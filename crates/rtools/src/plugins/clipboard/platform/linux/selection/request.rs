//! Scoped X11 selection conversions, including incremental transfers.

use super::{BYTE_LIMIT, Duration, Error, Result, Selection};
use std::time::Instant;
use x11rb::{
    connection::Connection as _,
    protocol::{
        Event,
        xproto::{Atom, AtomEnum, ConnectionExt as _, Property, Timestamp, Window},
    },
};

pub(super) struct Request<'a> {
    selection: &'a mut Selection,
    window: Window,
    time: Timestamp,
    active: &'a dyn Fn() -> bool,
}

impl<'a> Request<'a> {
    /// A fresh window per offer prevents delayed replies to an earlier request from being accepted.
    pub(super) fn new(
        selection: &'a mut Selection,
        time: Timestamp,
        active: &'a dyn Fn() -> bool,
    ) -> Result<Self> {
        let window = selection.display.window()?;
        Ok(Self {
            selection,
            window,
            time,
            active,
        })
    }

    /// Resolves the advertised atoms in batches without one server round trip per MIME type.
    pub(super) fn formats(&mut self) -> Result<Vec<String>> {
        // TARGETS contains only atom IDs. Bound the number of name lookups before reading it.
        let bytes = self.convert(
            self.selection.display.atoms.TARGETS,
            AtomEnum::ATOM.into(),
            4096 * 4,
        )?;
        let conn = &self.selection.display.conn;
        let mut names = Vec::new();
        for batch in bytes.as_chunks::<4>().0.chunks(64) {
            let cookies = batch
                .iter()
                .map(|id| conn.get_atom_name(u32::from_ne_bytes(*id)))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for cookie in cookies {
                if let Ok(name) = String::from_utf8(cookie.reply()?.name) {
                    names.push(name);
                }
            }
        }
        Ok(names)
    }

    /// Retrieves a representation previously advertised in TARGETS.
    pub(super) fn data(&mut self, name: &str) -> Result<Vec<u8>> {
        let target = self
            .selection
            .display
            .conn
            .intern_atom(true, name.as_bytes())?
            .reply()?
            .atom;
        if target == 0 {
            return Err(Error::Platform);
        }
        self.convert(target, target, BYTE_LIMIT)
    }

    /// Acknowledges each INCR chunk by deleting the property and stops on cancellation or a stalled source.
    fn convert(&mut self, target: Atom, kind: Atom, limit: usize) -> Result<Vec<u8>> {
        let display = &self.selection.display;
        let prop = display.atoms._RTOOLS_CLIPBOARD_TRANSFER;
        display
            .conn
            .convert_selection(self.window, display.atom, target, prop, self.time)?
            .check()?;
        let mut data = Vec::new();
        let mut incremental = false;
        let mut progress = Instant::now();
        while (self.active)() && progress.elapsed() < Duration::from_secs(5) {
            match display.poll_event()? {
                Some(Event::XfixesSelectionNotify(change)) if change.selection == display.atom => {
                    self.selection.pending = Some(change);
                    return Err(Error::Platform);
                }
                Some(Event::SelectionNotify(e))
                    if e.requestor == self.window
                        && e.selection == display.atom
                        && e.target == target =>
                {
                    if e.property != prop {
                        return Err(Error::Platform);
                    }
                }
                Some(Event::PropertyNotify(e))
                    if incremental
                        && e.window == self.window
                        && e.atom == prop
                        && e.state == Property::NEW_VALUE => {}
                _ => continue,
            }
            let remaining = limit - data.len();
            let reply = display
                .conn
                .get_property(
                    true,
                    self.window,
                    prop,
                    AtomEnum::ANY,
                    0,
                    remaining.div_ceil(4).max(1) as u32,
                )?
                .reply()?;
            if reply.bytes_after != 0 {
                return Err(Error::Limit);
            }
            if !incremental && reply.type_ == display.atoms.INCR {
                if reply
                    .value32()
                    .and_then(|mut sizes| sizes.next())
                    .is_some_and(|size| size as usize > limit)
                {
                    return Err(Error::Limit);
                }
                incremental = true;
            } else {
                // AtomEnum converts to several integer widths, so compare it as an X11 Atom.
                let format = if kind == Atom::from(AtomEnum::ATOM) {
                    32
                } else {
                    8
                };
                if reply.type_ != kind || reply.format != format {
                    return Err(Error::Platform);
                }
                if reply.value.len() > remaining {
                    return Err(Error::Limit);
                }
                if !incremental {
                    return Ok(reply.value);
                }
                if reply.value.is_empty() {
                    return Ok(data);
                }
                data.extend_from_slice(&reply.value);
            }
            progress = Instant::now();
        }
        Err(Error::Platform)
    }
}

impl Drop for Request<'_> {
    /// Releases the requestor even when its provider disappears or the subscription stops mid-transfer.
    fn drop(&mut self) {
        let conn = &self.selection.display.conn;
        if let Ok(cookie) = conn.destroy_window(self.window) {
            cookie.ignore_error();
            let _ = conn.flush();
        }
    }
}
