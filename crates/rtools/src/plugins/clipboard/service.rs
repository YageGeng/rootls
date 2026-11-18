//! Resident monitoring, generation barriers and clipboard ownership.

mod monitor;

pub(super) use monitor::{Monitor, Subscription, Ticket};

use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::{collections::HashMap, path::PathBuf};

use super::{
    Result,
    content::{Group, Input, Payload},
    history::{History, Selection},
    platform,
};
use crate::settings::SettingsStore;
use futures::{StreamExt as _, channel::mpsc as events};
use gpui_kit::{App, Global};

pub(super) enum Notice {
    Status(u64, &'static str),
    Issue(u64, &'static str),
    Saved(u64),
    Changed,
}

/// Database operations and publishes can outlive any individual plugin window.
pub(super) struct Core {
    pub history: History,
    monitor: Mutex<Weak<Monitor>>,
    // Serializes database writes and holds only digests that an older capture could reinsert.
    deletions: Mutex<HashMap<Vec<u8>, u64>>,
    deletion_epoch: AtomicU64,
    publish: Mutex<()>,
    serial: AtomicU64,
    exports: AtomicBool,
    events: events::UnboundedSender<Notice>,
}

impl Core {
    /// Shares history storage and one clipboard publisher across all windows and capture sessions.
    pub(super) fn new(history: History, events: events::UnboundedSender<Notice>) -> Self {
        Self {
            exports: AtomicBool::new(
                history
                    .directory()
                    .is_ok_and(|root| root.join("exports").exists()),
            ),
            history,
            monitor: Mutex::default(),
            deletions: Mutex::default(),
            deletion_epoch: AtomicU64::new(0),
            publish: Mutex::default(),
            serial: AtomicU64::new(0),
            events,
        }
    }

    /// Builds exports before publishing and only retires previous exports after success.
    pub fn copy(&self, id: i64, selection: Selection, confirmed: Option<&[i64]>) -> Result<()> {
        let _guard = self.publish.lock().unwrap_or_else(|e| e.into_inner());
        let prepared = self.history.prepare(id, selection, confirmed)?;
        let serial = self.serial.load(Ordering::SeqCst).wrapping_add(1);
        platform::write(&prepared.payload, serial)?;
        self.serial.store(serial, Ordering::SeqCst);
        if let Some(dir) = prepared.exports {
            let _owned = dir.keep();
            self.exports.store(true, Ordering::Release);
        }
        let keep = match &prepared.payload {
            Payload::Files(paths) => paths.as_slice(),
            _ => &[],
        };
        self.retire(keep);
        tracing::debug!("published clipboard history entry {}", id);
        Ok(())
    }

    /// Never removes a directory containing a path still advertised to a receiver.
    fn retire(&self, keep: &[PathBuf]) {
        if self.exports.load(Ordering::Acquire) {
            match self.history.clean_exports(keep) {
                Ok(remaining) => self.exports.store(remaining, Ordering::Release),
                Err(err) => tracing::warn!("could not retire clipboard exports: {}", err),
            }
        }
    }

    /// Serializes snapshots and routes storage outcomes separately from the listener status.
    fn store(
        &self,
        generation: &Arc<AtomicU64>,
        session: u64,
        rx: mpsc::Receiver<(Ticket, Vec<Input>)>,
    ) {
        for (ticket, inputs) in rx {
            if generation.load(Ordering::Acquire) != ticket.generation {
                continue;
            }
            if self.exports.load(Ordering::Acquire) {
                let _publish = self.publish.lock().unwrap_or_else(|e| e.into_inner());
                if self.serial.load(Ordering::SeqCst) == ticket.serial
                    && generation.load(Ordering::Acquire) == ticket.generation
                {
                    let keep = inputs
                        .iter()
                        .filter_map(|i| match i {
                            Input::File(p, _) => Some(p.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    self.retire(&keep);
                }
            }
            if inputs.is_empty() {
                continue;
            }
            let result = Group::try_from(inputs).and_then(|group| {
                let mut deleted = self.deletions.lock().unwrap_or_else(|e| e.into_inner());
                if generation.load(Ordering::Acquire) != ticket.generation {
                    return Ok(false);
                }
                // The active listener submits in FIFO order. Once it crosses a deletion,
                // its older captures have drained and the corresponding digest can be forgotten.
                deleted.retain(|_, cutoff| *cutoff > ticket.deletion_epoch);
                if deleted.contains_key(&group.digest) {
                    return Ok(false);
                }
                self.history.save(group, ticket.generation, generation)
            });
            match result {
                Ok(true) => {
                    let _ = self.events.unbounded_send(Notice::Saved(session));
                }
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!("clipboard capture failed: {}", err);
                    if generation.load(Ordering::Acquire) == ticket.generation {
                        let _ = self
                            .events
                            .unbounded_send(Notice::Issue(session, err.key()));
                    }
                }
            }
        }
    }

    /// Serializes clearing with capture commits and already-prepared clipboard exports.
    pub fn remove(&self, id: Option<i64>) -> Result<()> {
        let _publish = self.publish.lock().unwrap_or_else(|e| e.into_inner());
        let mut deleted = self.deletions.lock().unwrap_or_else(|e| e.into_inner());
        let monitor = match id {
            None => self
                .monitor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .upgrade(),
            Some(_) => None,
        };
        if let Some(run) = &monitor {
            run.interrupt();
        }
        if let Some(digest) = self.history.remove(id)? {
            let cutoff = self.deletion_epoch.fetch_add(1, Ordering::AcqRel) + 1;
            deleted.insert(digest, cutoff);
        } else if id.is_none() {
            deleted.clear();
        }
        let _ = self.events.unbounded_send(Notice::Changed);
        if let Some(id) = id {
            tracing::debug!("deleted clipboard history entry {}", id);
        } else {
            self.history.clean_exports(&[])?;
            self.exports.store(false, Ordering::Release);
            tracing::info!("cleared clipboard history");
        }
        // The new reader can fill its bounded queue while deletion runs. Let it drain
        // before waiting for its native baseline so the next copy after clear is observable.
        drop(deleted);
        drop(_publish);
        if let Some(run) = monitor {
            run.wait_for_baseline()?;
        }
        Ok(())
    }
}

/// The shell and all windows observe this same capture state.
pub struct ClipboardState {
    pub(super) core: Arc<Core>,
    phase: &'static str,
    issue: Option<&'static str>,
    pub(super) revision: u64,
    session: u64,
    monitor: Option<Arc<Monitor>>,
    enabled: bool,
}
impl Global for ClipboardState {}

impl ClipboardState {
    /// Installs the service after settings load and observes settings across every window.
    pub fn init(directory: Option<PathBuf>, cx: &mut App) {
        let history = History::new(directory);
        let (events, mut rx) = events::unbounded();
        cx.set_global(Self {
            core: Arc::new(Core::new(history, events)),
            phase: "clipboard.off",
            issue: None,
            revision: 0,
            session: 0,
            monitor: None,
            enabled: false,
        });
        cx.observe_global::<SettingsStore>(Self::configure).detach();
        cx.on_app_quit(|cx| {
            let state = cx.global_mut::<Self>();
            if let Some(run) = state.monitor.take() {
                run.stop();
            }
            async {}
        })
        .detach();
        cx.spawn(async move |cx| {
            while let Some(event) = rx.next().await {
                cx.update(|cx| {
                    let state = cx.global_mut::<Self>();
                    state.receive(event);
                });
            }
        })
        .detach();
        Self::configure(cx);
    }

    /// Only an active listener can display a transient capture error in place of its normal status.
    pub(crate) fn status(&self) -> &'static str {
        if self.phase == "clipboard.recording" {
            self.issue.unwrap_or(self.phase)
        } else {
            self.phase
        }
    }

    /// A stored offer refreshes history even when the originating listener has already exited.
    fn receive(&mut self, event: Notice) {
        match event {
            Notice::Changed => self.revision += 1,
            Notice::Saved(session) => {
                self.revision += 1;
                if self.session == session {
                    self.issue = None;
                }
            }
            Notice::Status(session, phase) if self.enabled && self.session == session => {
                self.phase = phase;
                self.issue = None;
            }
            Notice::Issue(session, reason) if self.enabled && self.session == session => {
                self.issue = Some(reason)
            }
            _ => {}
        }
    }

    /// Applies a changed preference exactly once, including settings-file reloads.
    fn configure(cx: &mut App) {
        let settings = cx.global::<SettingsStore>();
        let enabled = settings.settings.clipboard.enabled;
        let off = if settings.file.clipboard_save_failed {
            "clipboard.preference_error"
        } else {
            "clipboard.off"
        };
        if cx.global::<Self>().enabled == enabled {
            if !enabled && cx.global::<Self>().phase != off {
                cx.global_mut::<Self>().phase = off;
            }
            return;
        }
        let state = cx.global_mut::<Self>();
        state.enabled = enabled;
        state.session += 1;
        state.issue = None;
        if let Some(run) = state.monitor.take() {
            run.stop();
        }
        if enabled {
            if let Err(err) = state.core.history.directory() {
                tracing::warn!("could not start clipboard history: {}", err);
                state.phase = err.key();
                return;
            }
            state.phase = "clipboard.starting";
            match Monitor::start(Arc::clone(&state.core), state.session, platform::watch) {
                Ok(run) => {
                    state.monitor = Some(run);
                    tracing::info!("starting clipboard history monitor");
                }
                Err(e) => {
                    state.phase = "clipboard.unavailable";
                    tracing::warn!("could not start clipboard history monitor: {}", e);
                }
            }
        } else {
            state.phase = off;
            tracing::info!("stopped clipboard history monitor");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    /// Deleting an entry discards its older duplicate without losing other offers or a later recopy.
    #[test]
    fn single_delete_excludes_only_earlier_copies_of_that_entry() {
        let temp = tempfile::tempdir().unwrap();
        let (events, _) = events::unbounded();
        let core = Arc::new(Core::new(History::new(temp.path().to_owned()), events));
        let text = |s: &str| vec![Input::Text(s.to_owned())];
        core.history
            .save(
                Group::try_from(text("duplicate")).unwrap(),
                0,
                &Arc::default(),
            )
            .unwrap();
        let id = core.history.search("", false).unwrap()[0].id;
        let (acquired, ready) = mpsc::channel();
        let (advance, resume) = mpsc::channel();
        let run = Monitor::start(Arc::clone(&core), 1, move |sub| {
            let duplicate = sub.ticket();
            let unrelated = sub.ticket();
            acquired.send(()).unwrap();
            if resume.recv().is_ok() {
                sub.submit(duplicate, text("duplicate"));
                sub.submit(unrelated, text("unrelated"));
            }
            if resume.recv().is_ok() {
                sub.submit(sub.ticket(), text("duplicate"));
            }
            Ok(())
        })
        .unwrap();
        let wait_for = |query| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(row) = core.history.search(query, false).unwrap().first() {
                    break row.id;
                }
                assert!(Instant::now() < deadline, "capture never arrived");
                thread::sleep(Duration::from_millis(2));
            }
        };
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        core.remove(Some(id)).unwrap();
        advance.send(()).unwrap();
        wait_for("unrelated");
        assert!(core.history.search("duplicate", false).unwrap().is_empty());
        advance.send(()).unwrap();
        assert!(wait_for("duplicate") > id);
        run.stop();
    }

    /// The native reader may finish opening an old offer after a clear; only the next copy may commit.
    #[test]
    fn clearing_while_an_offer_is_being_read_keeps_only_the_next_copy() {
        let temp = tempfile::tempdir().unwrap();
        let (events, mut notices) = events::unbounded();
        let core = Arc::new(Core::new(History::new(temp.path().to_owned()), events));
        let (acquired, ready) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let mut initial = true;
        let run = Monitor::start(Arc::clone(&core), 1, move |listener| {
            let ticket = listener.ticket();
            listener.ready();
            if std::mem::take(&mut initial) {
                acquired.send(()).unwrap();
                resume.recv().unwrap();
                listener.submit(ticket, vec![Input::Text("old".into())]);
            } else {
                listener.submit(ticket, vec![Input::Text("new".into())]);
            }
            Ok(())
        })
        .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        thread::scope(|scope| {
            let clear = scope.spawn(|| core.remove(None));
            // Deletion has committed while the first native read is still in flight.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !matches!(notices.try_recv(), Ok(Notice::Changed)) {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(2));
            }
            release.send(()).unwrap();
            clear.join().unwrap().unwrap();
            while !matches!(notices.try_recv(), Ok(Notice::Saved(1))) {
                assert!(Instant::now() < deadline, "the new copy never committed");
                thread::sleep(Duration::from_millis(2));
            }
        });
        let rows = core.history.search("", false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "new");
        run.stop();
    }
    /// A reader holding back COMMIT must neither delay opt-out nor let the canceled capture persist.
    #[test]
    fn stopping_during_a_blocked_commit_returns_before_the_reader_releases() {
        let temp = tempfile::tempdir().unwrap();
        let (events, _) = events::unbounded();
        let core = Arc::new(Core::new(History::new(temp.path().to_owned()), events));
        core.history.search("", false).unwrap();
        let path = temp.path().join("history.sqlite3");
        let reader = rusqlite::Connection::open(&path).unwrap();
        reader
            .execute_batch("BEGIN; SELECT count(*) FROM history;")
            .unwrap();
        let probe = rusqlite::Connection::open(path).unwrap();
        probe.busy_timeout(Duration::ZERO).unwrap();
        let run = Monitor::start(Arc::clone(&core), 1, |run| {
            run.submit(run.ticket(), vec![Input::Text("pending".into())]);
            Ok(())
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        // SQLite's pending commit lock denies new readers while the original snapshot stays open.
        loop {
            match probe.query_row("SELECT count(*) FROM history", [], |r| r.get::<_, i64>(0)) {
                Err(e) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy) => break,
                Ok(_) => {}
                Err(err) => panic!("unexpected read failure: {err}"),
            }
            assert!(Instant::now() < deadline, "writer never reached COMMIT");
            thread::sleep(Duration::from_millis(2));
        }
        let start = Instant::now();
        run.stop();
        let elapsed = start.elapsed();
        reader.execute_batch("ROLLBACK").unwrap();
        let _finished = core.deletions.lock().unwrap();
        assert!(
            elapsed < Duration::from_millis(500),
            "stop waited {elapsed:?}"
        );
        assert!(core.history.search("", false).unwrap().is_empty());
    }

    /// A slow native sequence query on the clear worker must not hold up the settings thread.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn opt_out_does_not_wait_for_a_concurrent_native_baseline_read() {
        let temp = tempfile::tempdir().unwrap();
        let (events, mut notices) = events::unbounded();
        let core = Arc::new(Core::new(History::new(temp.path().to_owned()), events));
        let (entered, sampling) = mpsc::channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let native = Arc::clone(&release);
        let run = Monitor::start(Arc::clone(&core), 1, move |sub| {
            let native = Arc::clone(&native);
            let entered = entered.clone();
            sub.sequence_baseline(0, move || {
                entered.send(()).unwrap();
                native.wait();
                1
            });
            sub.ready();
            while sub.active() {
                thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !matches!(
            notices.try_recv(),
            Ok(Notice::Status(_, "clipboard.recording"))
        ) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        thread::scope(|scope| {
            let clear = scope.spawn(|| core.remove(None));
            sampling.recv_timeout(Duration::from_secs(3)).unwrap();
            let (stopped, done) = mpsc::channel();
            let run = &run;
            scope.spawn(move || {
                run.stop();
                let _ = stopped.send(());
            });
            let returned = done.recv_timeout(Duration::from_millis(500)).is_ok();
            release.wait();
            clear.join().unwrap().unwrap();
            assert!(returned, "opt-out waited for an OS clipboard query");
        });
    }

    /// Draining the last accepted offer must not revive a failed native subscription.
    #[test]
    fn queued_save_preserves_the_terminal_listener_status() {
        let temp = tempfile::tempdir().unwrap();
        let (events, mut notices) = events::unbounded();
        let core = Arc::new(Core::new(History::new(temp.path().to_owned()), events));
        let hold = core.deletions.lock().unwrap();
        let run = Monitor::start(Arc::clone(&core), 1, |run| {
            run.submit(run.ticket(), vec![Input::Text("last accepted copy".into())]);
            Err(super::super::Error::Platform)
        })
        .unwrap();
        let mut state = ClipboardState {
            core: Arc::clone(&core),
            phase: "clipboard.starting",
            issue: None,
            revision: 0,
            session: 1,
            monitor: Some(Arc::clone(&run)),
            enabled: true,
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while state.status() != "clipboard.unavailable" {
            if let Ok(event) = notices.try_recv() {
                state.receive(event);
            }
            assert!(Instant::now() < deadline, "listener did not report exit");
            thread::sleep(Duration::from_millis(2));
        }
        drop(hold);
        while state.revision == 0 {
            if let Ok(event) = notices.try_recv() {
                state.receive(event);
            }
            assert!(Instant::now() < deadline, "queued capture never finished");
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(state.status(), "clipboard.unavailable");
        assert_eq!(core.history.search("", false).unwrap().len(), 1);
        run.stop();
    }
}
