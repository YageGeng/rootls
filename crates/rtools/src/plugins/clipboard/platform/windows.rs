//! Clipboard format listener with a single OpenClipboard scope per multi-format offer.

use super::super::{
    Error, Result,
    content::{BYTE_LIMIT, Input, Payload},
    service::Subscription,
};
use super::OWNER;
use clipboard_win::{self as win, formats, options::NoClear, raw};
use image::ImageFormat;
use std::io::Cursor;
use std::path::PathBuf;
use std::{thread, time::Duration};

/// Checks private format markers while the clipboard is locked against concurrent replacement.
fn read(clipboard: win::Clipboard) -> Result<Option<Vec<Input>>> {
    let format = |name| {
        win::register_format(name)
            .map(|n| n.get())
            .ok_or(Error::Platform)
    };
    for name in [
        OWNER,
        "ExcludeClipboardContentFromMonitorProcessing",
        "Clipboard Viewer Ignore",
    ] {
        if win::is_format_avail(format(name)?) {
            return Ok(None);
        }
    }
    let history_flag = format("CanIncludeInClipboardHistory")?;
    if win::is_format_avail(history_flag) {
        let data: Vec<u8> =
            win::get(formats::RawData(history_flag)).map_err(|_| Error::Platform)?;
        if data.get(..4) != Some(&1u32.to_le_bytes()) {
            return Ok(None);
        }
    }
    if win::is_format_avail(formats::CF_HDROP) {
        let paths: Vec<PathBuf> = win::get(formats::FileList).map_err(|_| Error::Platform)?;
        drop(clipboard);
        return Ok(Some(paths.into_iter().map(Input::file).collect()));
    }
    let png = format("PNG")?;
    for ty in [png, formats::CF_DIBV5, formats::CF_DIB] {
        if !win::is_format_avail(ty) {
            continue;
        }
        if win::size(ty).is_some_and(|n| n.get() > BYTE_LIMIT) {
            return Err(Error::Limit);
        }
        let bytes: Vec<u8> = win::get(formats::RawData(ty)).map_err(|_| Error::Platform)?;
        return Ok(Some(vec![if ty == png {
            Input::Image(bytes)
        } else {
            Input::Dib(bytes)
        }]));
    }
    if win::is_format_avail(formats::CF_UNICODETEXT) {
        if win::size(formats::CF_UNICODETEXT).is_some_and(|n| n.get() > BYTE_LIMIT) {
            return Err(Error::Limit);
        }
        let text: String = win::get(win::Unicode).map_err(|_| Error::Platform)?;
        if !text.is_empty() {
            return Ok(Some(vec![Input::Text(text)]));
        }
    }
    Ok(Some(Vec::new()))
}

/// Waits on WM_CLIPBOARDUPDATE; the shutdown handle wakes an idle message loop.
pub(in crate::plugins::clipboard) fn watch(run: &Subscription<'_>) -> Result<()> {
    let mut monitor = win::Monitor::new().map_err(|_| Error::Platform)?;
    let shutdown = monitor.shutdown_channel();
    run.on_stop(move || drop(shutdown));
    let sequence = || win::seq_num().map_or(0, |n| u64::from(n.get()));
    let mut previous = run.sequence_baseline(sequence(), sequence);
    let mut busy_reported = false;
    run.ready();
    'watch: while run.active() {
        // A copy between clear and listener registration will not have posted a message to this window.
        let ticket = run.ticket();
        let seq = sequence();
        if seq != previous {
            // Unlocking the clipboard does not post another update. Keep the original ticket
            // while retrying, and abandon it as soon as a newer offer or a stop arrives.
            let mut delay = Duration::from_millis(5);
            let clipboard = loop {
                match win::Clipboard::new() {
                    Ok(clipboard) => break clipboard,
                    Err(err) => {
                        if !busy_reported && delay >= Duration::from_millis(80) {
                            tracing::warn!("waiting for the Windows clipboard to open: {}", err);
                            run.issue(Error::Platform.key());
                            busy_reported = true;
                        }
                        thread::sleep(delay);
                        delay = (delay * 2).min(Duration::from_millis(250));
                        if !run.active() || sequence() != seq {
                            continue 'watch;
                        }
                    }
                }
            };
            if std::mem::take(&mut busy_reported) {
                tracing::debug!("Windows clipboard is readable again");
                run.ready();
            }
            previous = seq;
            match read(clipboard) {
                Ok(Some(inputs)) if sequence() == seq => run.submit(ticket, inputs),
                Err(err) => {
                    tracing::warn!("Windows clipboard read failed: {}", err);
                    run.issue(err.key());
                }
                _ => {}
            }
            continue;
        }
        if !run.active() || !monitor.recv().map_err(|_| Error::Platform)? {
            break;
        }
    }
    Ok(())
}

/// Prepares both PNG and DIB before opening the clipboard; CF_HDROP carries the whole group.
pub(in crate::plugins::clipboard) fn write(payload: &Payload, _serial: u64) -> Result<()> {
    let image = if let Payload::Image(bytes) = payload {
        let png = Payload::png(bytes)?;
        let mut bmp = Vec::new();
        image::load_from_memory(&png)?.write_to(&mut Cursor::new(&mut bmp), ImageFormat::Bmp)?;
        Some((png, bmp))
    } else {
        None
    };
    let paths = if let Payload::Files(files) = payload {
        files
            .iter()
            .map(|p| p.to_str().ok_or(Error::Invalid))
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let own = win::register_format(OWNER).ok_or(Error::Platform)?.get();
    let png_type = win::register_format("PNG").ok_or(Error::Platform)?.get();
    // EmptyClipboard must assign a real owner before SetClipboardData can publish the formats.
    let owner = windows_win::Window::from_builder(
        windows_win::raw::window::Builder::new()
            .class_name("STATIC")
            .parent_message(),
    )
    .map_err(|_| Error::Platform)?;
    let _open = win::Clipboard::new_attempts_for(owner.inner(), 12).map_err(|_| Error::Platform)?;
    win::empty().map_err(|_| Error::Platform)?;
    raw::set_without_clear(own, &[1]).map_err(|_| Error::Platform)?;
    match payload {
        Payload::Text(text) => raw::set_string_with(text, NoClear),
        Payload::Files(_) => raw::set_file_list_with(&paths, NoClear),
        Payload::Image(_) => {
            let (png, bmp) = image.ok_or(Error::Invalid)?;
            raw::set_without_clear(png_type, &png).map_err(|_| Error::Platform)?;
            // The 14-byte BMP file header precedes the DIB consumed by Win32.
            raw::set_without_clear(formats::CF_DIB, bmp.get(14..).ok_or(Error::Invalid)?)
        }
    }
    .map_err(|_| Error::Platform)
}
