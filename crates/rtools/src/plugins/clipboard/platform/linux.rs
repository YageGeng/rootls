//! XFixes and Wayland data-control event listeners for the regular clipboard.

mod selection;

use super::super::{
    Error, Result,
    content::{BYTE_LIMIT, Input, Payload},
    service::Subscription,
};
use super::OWNER;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use selection::{Owner, Selection};
use std::{
    io::Read,
    os::fd::AsFd,
    thread,
    time::{Duration, Instant},
};
use url::Url;
use wl_clipboard_rs::{
    copy,
    paste::Seat,
    watch::{ClipboardEvent, ClipboardType, Watcher},
};

impl Subscription<'_> {
    /// Bounds a Wayland transfer and releases its pipe if the source stalls or capture is disabled.
    fn receive(&self, mut pipe: impl Read + AsFd) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let mut chunk = [0; 32768];
        let mut progress = Instant::now();
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        while self.active() && progress.elapsed() < Duration::from_secs(5) {
            match poll(&mut [PollFd::new(&pipe, PollFlags::IN)], Some(&timeout)) {
                Ok(0) | Err(rustix::io::Errno::INTR) => continue,
                Err(_) => return Err(Error::Platform),
                Ok(_) => {}
            }
            let n = pipe.read(&mut chunk)?;
            if n == 0 {
                return Ok(data);
            }
            if n > BYTE_LIMIT - data.len() {
                return Err(Error::Limit);
            }
            data.extend_from_slice(&chunk[..n]);
            progress = Instant::now();
        }
        Err(Error::Platform)
    }
}

/// Reads just one representation, giving file lists priority over their plain-text aliases.
fn offer(
    types: &[String],
    mut get: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<Option<Vec<Input>>> {
    let has = |name: &str| types.iter().any(|t| t.eq_ignore_ascii_case(name));
    if has(OWNER) {
        return Ok(None);
    }
    if has("x-kde-passwordManagerHint") {
        let hint = get("x-kde-passwordManagerHint")?;
        if hint.strip_suffix(&[0]).unwrap_or(&hint) == b"secret" {
            return Ok(None);
        }
    }
    for ty in [
        "text/uri-list",
        "x-special/gnome-copied-files",
        "image/png",
        "image/jpeg",
        "image/tiff",
        "image/webp",
        "image/bmp",
        "image/gif",
        "text/plain;charset=utf-8",
        "UTF8_STRING",
        "text/plain",
        "STRING",
    ] {
        let Some(actual) = types.iter().find(|s| s.eq_ignore_ascii_case(ty)) else {
            continue;
        };
        let data = get(actual)?;
        if data.len() > BYTE_LIMIT {
            return Err(Error::Limit);
        }
        if ty == "text/uri-list" || ty == "x-special/gnome-copied-files" {
            let paths = String::from_utf8_lossy(&data)
                .lines()
                .filter_map(|s| Url::parse(s.trim()).ok()?.to_file_path().ok())
                .map(Input::file)
                .collect::<Vec<_>>();
            if !paths.is_empty() {
                return Ok(Some(paths));
            }
        } else if ty.starts_with("image/") {
            return Ok(Some(vec![Input::Image(data)]));
        } else if !data.is_empty() {
            return Ok(Some(vec![Input::Text(
                String::from_utf8_lossy(&data).into_owned(),
            )]));
        }
    }
    Ok(Some(Vec::new()))
}

/// Handles one native transfer failure without terminating monitoring of future offers.
fn deliver(
    run: &Subscription<'_>,
    ticket: super::super::service::Ticket,
    result: Result<Option<Vec<Input>>>,
) {
    match result {
        Ok(Some(inputs)) => run.submit(ticket, inputs),
        Ok(None) => {}
        Err(err) => {
            tracing::warn!("Linux clipboard transfer failed: {}", err);
            run.issue(err.key());
        }
    }
}

/// Selects the actual desktop protocol rather than silently monitoring only XWayland clients.
pub(in crate::plugins::clipboard) fn watch(run: &Subscription<'_>) -> Result<()> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let mut observed_publication = run.ticket().serial;
        let mut watcher =
            Watcher::new(ClipboardType::Regular, Seat::Unspecified).map_err(|_| Error::Platform)?;
        let cancel = watcher.cancel_handle();
        run.on_stop(move || cancel.cancel());
        // The first data-control event describes the selection from before opt-in.
        if watcher.next_event().map_err(|_| Error::Platform)?.is_none() {
            return Ok(());
        }
        run.ready();
        let replay_prefix = format!("{OWNER};serial=");
        while run.active() {
            let Some(event) = watcher.next_event().map_err(|_| Error::Platform)? else {
                break;
            };
            if !run.active() {
                break;
            }
            let mut ticket = run.ticket();
            // Queued offers preceding our newest publish must never retire its staged files.
            ticket.serial = observed_publication;
            match event {
                ClipboardEvent::Changed {
                    mime_types,
                    offer: mut source,
                    ..
                } => {
                    if let Some(serial) = mime_types.iter().find_map(|ty| {
                        ty.strip_prefix(&replay_prefix)
                            .and_then(|value| value.parse().ok())
                    }) {
                        observed_publication = serial;
                        continue;
                    }
                    let result = offer(&mime_types, |ty| {
                        run.receive(source.receive(ty).map_err(|_| Error::Platform)?)
                    });
                    deliver(run, ticket, result);
                }
                ClipboardEvent::Cleared { .. } => run.submit(ticket, Vec::new()),
            }
        }
        return Ok(());
    }
    let mut selection = Selection::subscribe(b"CLIPBOARD")?;
    run.ready();
    while run.active() {
        let Some(change) = selection.next_change()? else {
            thread::sleep(Duration::from_millis(35));
            continue;
        };
        let ticket = run.ticket();
        let result = selection.read(change, || run.active());
        if selection.is_current(change)? {
            deliver(run, ticket, result);
        }
    }
    Ok(())
}

/// Keeps the X11 owner alive; Wayland's copy worker serves requests after publication.
pub(in crate::plugins::clipboard) fn write(payload: &Payload, serial: u64) -> Result<()> {
    let (mime, data) = match payload {
        Payload::Text(text) => ("text/plain;charset=utf-8", text.as_bytes().to_vec()),
        Payload::Image(bytes) => ("image/png", Payload::png(bytes)?),
        Payload::Files(paths) => {
            let uris = paths
                .iter()
                .map(|p| {
                    Url::from_file_path(p)
                        .map(String::from)
                        .map_err(|_| Error::Invalid)
                })
                .collect::<Result<Vec<_>>>()?;
            ("text/uri-list", uris.join("\r\n").into_bytes())
        }
    };
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let sources = [
            (mime.to_owned(), data),
            (format!("{OWNER};serial={serial}"), vec![1]),
        ]
        .into_iter()
        .map(|(ty, bytes)| copy::MimeSource {
            source: copy::Source::Bytes(bytes.into()),
            mime_type: copy::MimeType::Specific(ty),
        })
        .collect();
        return copy::Options::new()
            .copy_multi(sources)
            .map_err(|_| Error::Platform);
    }
    Owner::publish(
        b"CLIPBOARD",
        mime,
        data,
        matches!(payload, Payload::Text(_)),
    )
}
