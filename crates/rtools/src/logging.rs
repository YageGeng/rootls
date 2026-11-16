//! Bounded, per-user diagnostics for GUI launches that have no console.

use std::{fs, io, io::IsTerminal as _, path::PathBuf};

use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, prelude::*};

/// Owns the platform-specific directory used by the rotating diagnostic writer.
pub struct LogFiles {
    directory: PathBuf,
}

impl LogFiles {
    /// Resolves a persistent user location without falling back to a shared temporary directory.
    pub fn new() -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        let directory = dirs::home_dir().map(|home| home.join("Library/Logs/rtools"));
        #[cfg(target_os = "linux")]
        let directory = dirs::state_dir().map(|state| state.join("rtools/logs"));
        #[cfg(target_os = "windows")]
        let directory = dirs::data_local_dir().map(|data| data.join("rtools/logs"));
        Ok(Self {
            directory: directory.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "user log directory is unavailable")
            })?,
        })
    }

    /// Opens a private log directory and retains at most seven daily diagnostic files.
    fn writer(&self) -> io::Result<RollingFileAppender> {
        fs::create_dir_all(&self.directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))?;
        }
        RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("rtools")
            .filename_suffix("log")
            .max_log_files(7)
            .build(&self.directory)
            .map_err(io::Error::other)
    }

    /// Installs file and stderr subscribers; retain the guard until main returns to flush errors.
    pub fn init(self) -> io::Result<WorkerGuard> {
        let (writer, guard) = tracing_appender::non_blocking(self.writer()?);
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("warn,rtools=info"));
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(writer),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(io::stderr().is_terminal())
                    .with_writer(io::stderr),
            )
            .try_init()
            .map_err(io::Error::other)?;
        tracing::info!("diagnostic logs: {}", self.directory.display());
        Ok(guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GUI-only errors must reach disk and be flushed when the application guard is dropped.
    #[test]
    fn errors_are_persisted_without_a_console() {
        let directory = tempfile::tempdir().unwrap();
        let logs = LogFiles {
            directory: directory.path().join("logs"),
        };
        let (writer, guard) = tracing_appender::non_blocking(logs.writer().unwrap());
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(writer)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("startup registration failed: test error");
        });
        drop(guard);
        let path = fs::read_dir(&logs.directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("startup registration failed: test error"));
        assert!(!text.contains('\u{1b}'));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&logs.directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
}
