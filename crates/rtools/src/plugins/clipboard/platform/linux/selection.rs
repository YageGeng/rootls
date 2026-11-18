//! X11 selection changes and native transfers over connections owned by the recorder.

mod owner;
mod request;

use super::{BYTE_LIMIT, Error, Input, Result};
pub(super) use owner::Owner;
use request::Request;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::time::Duration;
use x11rb::{
    connection::Connection as _,
    errors::{ConnectionError, ReplyError},
    protocol::{
        Event,
        xfixes::{self, SelectionNotifyEvent},
        xproto::{Atom, ConnectionExt as _, CreateWindowAux, EventMask, Window, WindowClass},
    },
    rust_connection::RustConnection,
};

x11rb::atom_manager! {
    Atoms: AtomCookies { TARGETS, INCR, UTF8_STRING, _RTOOLS_CLIPBOARD_TRANSFER, }
}

impl From<ConnectionError> for Error {
    /// Reports transport failures without including clipboard payloads.
    fn from(err: ConnectionError) -> Self {
        ReplyError::from(err).into()
    }
}

impl From<ReplyError> for Error {
    /// Maps a failed native request to the recorder's localized platform error.
    fn from(err: ReplyError) -> Self {
        tracing::debug!("X11 clipboard request failed: {}", err);
        Self::Platform
    }
}

/// Atoms are local to the display and the connection owns all windows it creates.
struct Display {
    conn: RustConnection,
    root: Window,
    atom: Atom,
    atoms: Atoms,
}

impl Display {
    /// Resolves the selection and the protocol atoms once for this connection.
    fn open(name: &[u8]) -> Result<Self> {
        let (conn, screen) = x11rb::connect(None).map_err(|err| {
            tracing::warn!("could not connect to the X11 clipboard display: {}", err);
            Error::Platform
        })?;
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn)?;
        let atom = conn.intern_atom(false, name)?.reply()?.atom;
        let atoms = atoms.reply()?;
        Ok(Self {
            conn,
            root,
            atom,
            atoms,
        })
    }

    /// Creates a private requestor or owner window; no window is needed just to observe changes.
    fn window(&self) -> Result<Window> {
        let id = self.conn.generate_id().map_err(|err| {
            tracing::warn!("could not allocate an X11 clipboard window: {}", err);
            Error::Platform
        })?;
        self.conn
            .create_window(
                0,
                id,
                self.root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )?
            .check()?;
        Ok(id)
    }

    /// Drains buffered events before polling the socket, waking periodically for cancellation and expiry.
    fn poll_event(&self) -> Result<Option<Event>> {
        if let Some(event) = self.conn.poll_for_event()? {
            return Ok(Some(event));
        }
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        match poll(
            &mut [PollFd::new(self.conn.stream(), PollFlags::IN)],
            Some(&timeout),
        ) {
            Ok(0) | Err(rustix::io::Errno::INTR) => Ok(None),
            Ok(_) => Ok(self.conn.poll_for_event()?),
            Err(err) => {
                tracing::debug!("could not poll the X11 clipboard connection: {}", err);
                Err(Error::Platform)
            }
        }
    }
}

pub(super) struct Selection {
    display: Display,
    pending: Option<SelectionNotifyEvent>,
}

impl Selection {
    /// Subscribes before the first read so subsequent ownership changes can be observed in order.
    pub fn subscribe(name: &[u8]) -> Result<Self> {
        let display = Display::open(name)?;
        xfixes::query_version(&display.conn, 5, 0)?.reply()?;
        xfixes::select_selection_input(
            &display.conn,
            display.root,
            display.atom,
            xfixes::SelectionEventMask::SET_SELECTION_OWNER
                | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
                | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?
        .check()?;
        Ok(Self {
            display,
            pending: None,
        })
    }

    /// Coalesces pending ownership changes while allowing an idle listener to stop.
    pub fn next_change(&mut self) -> Result<Option<SelectionNotifyEvent>> {
        let mut latest = self.pending.take();
        while let Some(event) = self.display.conn.poll_for_event()? {
            if let Event::XfixesSelectionNotify(change) = event
                && change.selection == self.display.atom
            {
                latest = Some(change);
            }
        }
        Ok(latest)
    }

    /// Reads one offer through a scoped requestor, checking privacy markers before requesting data.
    pub fn read(
        &mut self,
        change: SelectionNotifyEvent,
        active: impl Fn() -> bool,
    ) -> Result<Option<Vec<Input>>> {
        if change.owner == 0 {
            return Ok(Some(Vec::new()));
        }
        let mut request = Request::new(self, change.selection_timestamp, &active)?;
        let formats = request.formats()?;
        super::offer(&formats, |name| request.data(name))
    }

    /// Fences completed reads against all subsequent XFixes notifications, even with reused timestamps.
    pub fn is_current(&mut self, offer: SelectionNotifyEvent) -> Result<bool> {
        let owner = self
            .display
            .conn
            .get_selection_owner(self.display.atom)?
            .reply()?
            .owner;
        self.pending = self.next_change()?;
        Ok(self.pending.is_none() && owner == offer.owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::{
        CURRENT_TIME,
        protocol::xproto::{CreateWindowAux, WindowClass},
    };

    /// A large selection and a subsequent text offer must both reach their receivers intact.
    #[test]
    #[ignore = "requires an X server with XFixes"]
    fn incremental_paste_then_text_preserves_data_and_replay_markers() {
        let name = format!("_RTOOLS_TRANSFER_{}", std::process::id());
        let mut selection = Selection::subscribe(name.as_bytes()).unwrap();
        let bytes: Vec<u8> = (0..24 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        Owner::publish(
            name.as_bytes(),
            "application/octet-stream",
            bytes.clone(),
            false,
        )
        .unwrap();
        selection
            .display
            .conn
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap();
        let change = selection.next_change().unwrap().unwrap();
        {
            let mut request =
                Request::new(&mut selection, change.selection_timestamp, &|| true).unwrap();
            assert!(
                request
                    .formats()
                    .unwrap()
                    .iter()
                    .any(|s| s == crate::plugins::clipboard::platform::OWNER)
            );
            assert_eq!(
                request.data("application/octet-stream").unwrap().as_slice(),
                bytes
            );
        }
        assert!(selection.is_current(change).unwrap());
        Owner::publish(
            name.as_bytes(),
            "text/plain;charset=utf-8",
            b"next".to_vec(),
            true,
        )
        .unwrap();
        selection
            .display
            .conn
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap();
        let change = selection.next_change().unwrap().unwrap();
        let mut request =
            Request::new(&mut selection, change.selection_timestamp, &|| true).unwrap();
        assert_eq!(request.data("UTF8_STRING").unwrap(), b"next");
        assert_eq!(request.data("text/plain;charset=utf-8").unwrap(), b"next");
        drop(request);
        // Release the test owner without touching the desktop's CLIPBOARD selection.
        selection
            .display
            .conn
            .set_selection_owner(0u32, selection.display.atom, CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
    }

    /// Neither a stopped subscription nor a completed capture leaves a requestor on the server.
    #[test]
    #[ignore = "requires an X server with XFixes"]
    fn subscriptions_release_all_their_requestor_windows() {
        let (probe, screen) = x11rb::connect(None).unwrap();
        let root = probe.setup().roots[screen].root;
        for _ in 0..4 {
            let mut selection = Selection::subscribe(b"_RTOOLS_REQUESTOR_LIFETIME").unwrap();
            let setup = selection.display.conn.setup();
            let (base, mask) = (setup.resource_id_base, setup.resource_id_mask);
            let windows = || {
                probe
                    .query_tree(root)
                    .unwrap()
                    .reply()
                    .unwrap()
                    .children
                    .into_iter()
                    .filter(|id| id & !mask == base)
                    .count()
            };
            let request = Request::new(&mut selection, CURRENT_TIME, &|| true).unwrap();
            assert_eq!(windows(), 1);
            drop(request);
            selection
                .display
                .conn
                .get_input_focus()
                .unwrap()
                .reply()
                .unwrap();
            drop(selection);
            assert_eq!(windows(), 0);
        }
    }

    /// Repeated claims by the same window must invalidate the old offer and retain the next event.
    #[test]
    #[ignore = "requires an X server with XFixes"]
    fn same_owner_replacement_invalidates_the_transfer_and_preserves_the_new_offer() {
        let mut selection =
            Selection::subscribe(format!("_RTOOLS_TEST_{}", std::process::id()).as_bytes())
                .unwrap();
        let (publisher, screen) = x11rb::connect(None).unwrap();
        let root = publisher.setup().roots[screen].root;
        let window = publisher.generate_id().unwrap();
        publisher
            .create_window(
                0,
                window,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        publisher
            .set_selection_owner(window, selection.display.atom, CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        selection
            .display
            .conn
            .get_input_focus()
            .unwrap()
            .reply()
            .unwrap();
        let first = selection.next_change().unwrap().unwrap();
        assert_eq!(first.owner, window);
        // X11 timestamps have millisecond precision. Two offers may share both owner and timestamp.
        publisher
            .set_selection_owner(window, selection.display.atom, first.selection_timestamp)
            .unwrap()
            .check()
            .unwrap();
        assert!(
            !selection.is_current(first).unwrap(),
            "accepted the superseded transfer"
        );
        let next = selection
            .next_change()
            .unwrap()
            .expect("lost replacement notification");
        assert_eq!(
            (next.owner, next.selection_timestamp),
            (window, first.selection_timestamp)
        );
        assert!(selection.is_current(next).unwrap());
        assert!(selection.next_change().unwrap().is_none());
    }
}
