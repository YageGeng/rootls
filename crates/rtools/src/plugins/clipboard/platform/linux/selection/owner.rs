//! Serves one immutable X11 publication until ownership and outstanding transfers end.

use super::{Display, Error, Result};
use crate::plugins::clipboard::platform::OWNER;
use std::{
    collections::HashMap,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    CURRENT_TIME, NONE,
    connection::{Connection as _, RequestConnection as _},
    protocol::{
        Event,
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, PropMode,
            Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window,
        },
    },
    wrapper::ConnectionExt as _,
};

type Destination = (Window, Atom);

struct Transfer {
    target: Atom,
    offset: usize,
    progress: Instant,
}

pub(in crate::plugins::clipboard) struct Owner {
    display: Display,
    window: Window,
    target: Atom,
    marker: Atom,
    text: bool,
    data: Vec<u8>,
    chunk: usize,
    transfers: HashMap<Destination, Transfer>,
}

impl Owner {
    /// Starts the server before claiming the selection and acknowledges actual ownership to the caller.
    pub(in crate::plugins::clipboard) fn publish(
        name: &[u8],
        mime: &str,
        data: Vec<u8>,
        text: bool,
    ) -> Result<()> {
        let display = Display::open(name)?;
        let window = display.window()?;
        let target = display
            .conn
            .intern_atom(false, mime.as_bytes())?
            .reply()?
            .atom;
        let marker = display
            .conn
            .intern_atom(false, OWNER.as_bytes())?
            .reply()?
            .atom;
        // Leave room for the ChangeProperty header and avoid large copies on the X server.
        let chunk = display
            .conn
            .maximum_request_bytes()
            .saturating_sub(32)
            .clamp(1, 4 * 1024 * 1024);
        let mut owner = Self {
            display,
            window,
            target,
            marker,
            text,
            data,
            chunk,
            transfers: HashMap::new(),
        };
        let (started, ready) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("rtools-x11-clipboard-owner".into())
            .spawn(move || {
                let result = owner.claim();
                let claimed = result.is_ok();
                if started.send(result).is_ok()
                    && claimed
                    && let Err(err) = owner.serve()
                {
                    tracing::warn!("X11 clipboard owner stopped: {}", err);
                }
            })?;
        ready.recv().map_err(|_| Error::Platform)?
    }

    /// Confirms the server granted ownership before existing exports are retired.
    fn claim(&self) -> Result<()> {
        let x = &self.display;
        x.conn
            .set_selection_owner(self.window, x.atom, CURRENT_TIME)?
            .check()?;
        if x.conn.get_selection_owner(x.atom)?.reply()?.owner != self.window {
            tracing::warn!("X11 clipboard ownership changed during publication");
            return Err(Error::Platform);
        }
        Ok(())
    }

    /// Isolates each requestor's failures and finishes existing INCR transfers after ownership changes.
    fn serve(&mut self) -> Result<()> {
        let mut owns_selection = true;
        let mut last_cleanup = Instant::now();
        while owns_selection || !self.transfers.is_empty() {
            let event = if self.transfers.is_empty() {
                Some(self.display.conn.wait_for_event()?)
            } else {
                self.display.poll_event()?
            };
            match event {
                Some(Event::SelectionClear(_)) => owns_selection = false,
                Some(Event::SelectionRequest(request)) if owns_selection => {
                    // Obsolete clients can omit the property and expect the target atom to be used.
                    let prop = if request.property == NONE {
                        request.target
                    } else {
                        request.property
                    };
                    let key = (request.requestor, prop);
                    if let Err(err) = self.respond(request, key) {
                        tracing::debug!("X11 clipboard receiver left before notification: {}", err);
                        self.finish(key);
                    }
                }
                Some(Event::PropertyNotify(e)) if e.state == Property::DELETE => {
                    let key = (e.window, e.atom);
                    if let Err(err) = self.advance(key) {
                        tracing::debug!("X11 clipboard receiver left during transfer: {}", err);
                        self.finish(key);
                    }
                }
                Some(Event::DestroyNotify(e)) => self.transfers.retain(|(w, _), _| *w != e.window),
                _ => {}
            }
            // A receiver may stop deleting its property. Stop watching that window after a quiet period.
            if !self.transfers.is_empty() && last_cleanup.elapsed() >= Duration::from_secs(1) {
                let expired: Vec<_> = self
                    .transfers
                    .iter()
                    .filter(|(_, t)| t.progress.elapsed() >= Duration::from_secs(5))
                    .map(|(key, _)| *key)
                    .collect();
                for key in expired {
                    self.finish(key);
                }
                last_cleanup = Instant::now();
            }
        }
        Ok(())
    }

    /// Reports both successful and refused conversions to the requesting application.
    fn respond(&mut self, request: SelectionRequestEvent, key: Destination) -> Result<()> {
        let property = match self.answer(key, request.target) {
            Ok(true) => key.1,
            Ok(false) => NONE,
            Err(err) => {
                self.finish(key);
                tracing::debug!("could not serve X11 clipboard target: {}", err);
                NONE
            }
        };
        let reply = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: key.0,
            selection: request.selection,
            target: request.target,
            property,
        };
        self.display
            .conn
            .send_event(false, key.0, EventMask::NO_EVENT, reply)?
            .check()?;
        Ok(())
    }

    /// Sends small values directly or registers a deletion-driven incremental conversion.
    fn answer(&mut self, key: Destination, target: Atom) -> Result<bool> {
        let (window, property) = key;
        let x = &self.display;
        if self.transfers.contains_key(&key) {
            return Ok(false);
        }
        if target == x.atoms.TARGETS {
            let mut available = vec![x.atoms.TARGETS, self.marker, self.target];
            if self.text {
                available.push(x.atoms.UTF8_STRING);
            }
            x.conn
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    property,
                    AtomEnum::ATOM,
                    &available,
                )?
                .check()?;
        } else if target == self.marker {
            x.conn
                .change_property8(PropMode::REPLACE, window, property, target, &[1])?
                .check()?;
        } else if target != self.target && !(self.text && target == x.atoms.UTF8_STRING) {
            return Ok(false);
        } else if self.data.len() <= self.chunk {
            x.conn
                .change_property8(PropMode::REPLACE, window, property, target, &self.data)?
                .check()?;
        } else {
            let events = EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY;
            x.conn
                .change_window_attributes(
                    window,
                    &ChangeWindowAttributesAux::new().event_mask(events),
                )?
                .check()?;
            self.transfers.insert(
                key,
                Transfer {
                    target,
                    offset: 0,
                    progress: Instant::now(),
                },
            );
            x.conn
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    property,
                    x.atoms.INCR,
                    &[self.data.len() as u32],
                )?
                .check()?;
        }
        Ok(true)
    }

    /// Sends one negotiated chunk, followed by a zero-length property to mark successful completion.
    fn advance(&mut self, key: Destination) -> Result<()> {
        let Some(transfer) = self.transfers.get_mut(&key) else {
            return Ok(());
        };
        let end = (transfer.offset + self.chunk).min(self.data.len());
        let bytes = &self.data[transfer.offset..end];
        self.display
            .conn
            .change_property8(PropMode::REPLACE, key.0, key.1, transfer.target, bytes)?
            .check()?;
        if bytes.is_empty() {
            self.finish(key);
        } else {
            transfer.offset = end;
            transfer.progress = Instant::now();
        }
        Ok(())
    }

    /// Removes this client's event mask once all conversions on its window have ended.
    fn finish(&mut self, key: Destination) {
        if self.transfers.remove(&key).is_some()
            && !self.transfers.keys().any(|(w, _)| *w == key.0)
            && let Ok(cookie) = self.display.conn.change_window_attributes(
                key.0,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            )
        {
            cookie.ignore_error();
            let _ = self.display.conn.flush();
        }
    }
}
