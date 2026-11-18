//! Native subscription lifecycle and capture generations.

use super::{Core, Notice};
use crate::plugins::clipboard::{Error, Result, content::Input};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::{thread, time::Duration};

/// An enabled recording session survives native resubscriptions after a clear.
pub(in crate::plugins::clipboard) struct Monitor {
    enabled: AtomicBool,
    generation: Arc<AtomicU64>,
    listener: Mutex<ListenerState>,
    baseline: Condvar,
    session: u64,
    core: Arc<Core>,
    input: mpsc::SyncSender<(Ticket, Vec<Input>)>,
}

#[derive(Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Starting,
    Ready,
    Ended,
}

#[derive(Default)]
struct ListenerState {
    phase: Phase,
    cancel: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    counter: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    clear_sequence: Option<u64>,
}

/// Every event from one native subscription keeps its original generation, including queued events.
pub(in crate::plugins::clipboard) struct Subscription<'a> {
    monitor: &'a Monitor,
    generation: u64,
}

#[derive(Clone, Copy)]
pub(in crate::plugins::clipboard) struct Ticket {
    pub(super) generation: u64,
    pub(super) deletion_epoch: u64,
    pub(in crate::plugins::clipboard) serial: u64,
}

impl Subscription<'_> {
    /// Captures the publication serial before opening an offer or requesting native data.
    pub fn ticket(&self) -> Ticket {
        Ticket {
            generation: self.generation,
            deletion_epoch: self.monitor.core.deletion_epoch.load(Ordering::Acquire),
            serial: self.monitor.core.serial.load(Ordering::SeqCst),
        }
    }

    /// Stops reads from retired subscriptions immediately on opt-out or clear.
    pub fn active(&self) -> bool {
        self.monitor.enabled.load(Ordering::Acquire)
            && self.monitor.generation.load(Ordering::Acquire) == self.generation
    }

    /// Wakes a blocked native wait even if cancellation raced watcher initialization.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub fn on_stop(&self, cancel: impl FnOnce() + Send + 'static) {
        let mut listener = self
            .monitor
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.active() {
            listener.cancel = Some(Box::new(cancel));
        } else {
            drop(listener);
            cancel();
        }
    }

    /// Carries the sequence sampled at clear across reopening, including intervening copies.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn sequence_baseline(
        &self,
        current: u64,
        counter: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> u64 {
        let mut listener = self
            .monitor
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !self.active() {
            return current;
        }
        listener.counter = Some(Arc::new(counter));
        listener.clear_sequence.take().unwrap_or(current)
    }

    /// Acknowledges that the platform has excluded the selection predating this subscription.
    pub fn ready(&self) {
        let mut listener = self
            .monitor
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.active() {
            listener.phase = Phase::Ready;
            let _ = self
                .monitor
                .core
                .events
                .unbounded_send(Notice::Status(self.monitor.session, "clipboard.recording"));
            self.monitor.baseline.notify_all();
        }
    }

    /// Reports a recoverable transfer error without changing the native listener's lifecycle.
    pub fn issue(&self, key: &'static str) {
        if self.active() {
            let _ = self
                .monitor
                .core
                .events
                .unbounded_send(Notice::Issue(self.monitor.session, key));
        }
    }

    /// Hands a stable offer to the worker without allowing an old subscription to acquire a new ticket.
    pub fn submit(&self, ticket: Ticket, inputs: Vec<Input>) {
        if self.active() {
            let _ = self.monitor.input.send((ticket, inputs));
        }
    }
}

impl Monitor {
    /// Invalidates queued captures and wakes native listeners without waiting for database writers.
    pub(super) fn interrupt(&self) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let clear_sequence = if self.enabled.load(Ordering::Acquire) {
            let counter = self
                .listener
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .counter
                .as_ref()
                .map(Arc::clone);
            // The OS can stall. Release the lifecycle lock before sampling on the clear worker.
            counter.map(|sample| sample())
        } else {
            None
        };
        let mut listener = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            listener.clear_sequence = clear_sequence;
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        if listener.phase != Phase::Ended {
            listener.phase = Phase::Starting;
            if self.enabled.load(Ordering::Acquire) {
                let _ = self
                    .core
                    .events
                    .unbounded_send(Notice::Status(self.session, "clipboard.starting"));
                tracing::debug!("restarting clipboard subscription after clear");
            }
        }
        let cancel = listener.cancel.take();
        self.baseline.notify_all();
        drop(listener);
        if let Some(cancel) = cancel {
            cancel();
        }
    }

    /// Stops capture immediately; SQLite checks the invalidated generation before completing a commit.
    pub(in crate::plugins::clipboard) fn stop(&self) {
        if self.enabled.swap(false, Ordering::AcqRel) {
            self.interrupt();
        }
    }

    /// Completes clear after a fresh baseline, a concurrent stop, or a known native failure.
    pub(super) fn wait_for_baseline(&self) -> Result<()> {
        let state = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        let (_state, timeout) = self
            .baseline
            .wait_timeout_while(state, Duration::from_secs(5), |s| {
                self.enabled.load(Ordering::Acquire) && s.phase == Phase::Starting
            })
            .unwrap_or_else(|e| e.into_inner());
        if timeout.timed_out() {
            tracing::warn!("clipboard history was cleared but native resubscription timed out");
            return Err(Error::Platform);
        }
        Ok(())
    }

    /// Reopens the native watcher on clear; an unsolicited EOF is terminal just like an error.
    fn listen(&self, mut watch: impl FnMut(&Subscription<'_>) -> Result<()>) {
        while self.enabled.load(Ordering::Acquire) {
            let subscription = Subscription {
                monitor: self,
                generation: self.generation.load(Ordering::Acquire),
            };
            let result = watch(&subscription);
            let mut state = self.listener.lock().unwrap_or_else(|e| e.into_inner());
            if !subscription.active() {
                continue;
            }
            state.phase = Phase::Ended;
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                state.counter = None;
            }
            let cancel = state.cancel.take();
            match result {
                Ok(()) => tracing::warn!("clipboard listener ended unexpectedly"),
                Err(err) => tracing::warn!("clipboard listener stopped: {}", err),
            }
            let _ = self
                .core
                .events
                .unbounded_send(Notice::Status(self.session, "clipboard.unavailable"));
            self.baseline.notify_all();
            drop(state);
            drop(cancel);
            break;
        }
    }

    /// Starts a worker and its native subscription without opening a window.
    pub(in crate::plugins::clipboard) fn start(
        core: Arc<Core>,
        session: u64,
        watch: impl FnMut(&Subscription<'_>) -> Result<()> + Send + 'static,
    ) -> std::io::Result<Arc<Self>> {
        let (input, rx) = mpsc::sync_channel(2);
        let monitor = Arc::new(Self {
            enabled: AtomicBool::new(true),
            generation: Arc::new(AtomicU64::new(0)),
            listener: Mutex::default(),
            baseline: Condvar::new(),
            session,
            core: Arc::clone(&core),
            input,
        });
        // A concurrent clear must see this monitor once its native subscription can observe offers.
        let mut current = core.monitor.lock().unwrap_or_else(|e| e.into_inner());
        let worker = Arc::clone(&core);
        let generation = Arc::clone(&monitor.generation);
        thread::Builder::new()
            .name("rtools-clipboard-store".into())
            .spawn(move || worker.store(&generation, session, rx))?;
        let listener = Arc::clone(&monitor);
        thread::Builder::new()
            .name("rtools-clipboard-monitor".into())
            .spawn(move || listener.listen(watch))?;
        *current = Arc::downgrade(&monitor);
        Ok(monitor)
    }
}
