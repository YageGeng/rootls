//! Current-user Run registration; never modifies machine-wide startup settings.

use std::{io, path::PathBuf};
use windows_registry::{CURRENT_USER, Type};

use super::Status;
use crate::{cli::ShellCommand, shell::APP_ID};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// Owns the executable whose login command is registered for the current user.
pub(crate) struct Autostart {
    executable: PathBuf,
}

impl Autostart {
    /// Resolves the installed executable without reading or writing the registry.
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            executable: std::env::current_exe()?,
        })
    }

    /// Reads Run and the Task Manager override so an externally disabled entry stays disabled.
    pub(crate) fn status(&self) -> io::Result<Status> {
        match CURRENT_USER
            .open(RUN)
            .and_then(|key| key.get_string(APP_ID))
            .map_err(io::Error::from)
        {
            Ok(command) if command.is_empty() => return Ok(Status::Disabled),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Status::Disabled),
            Err(error) => return Err(error),
        }
        match CURRENT_USER
            .open(APPROVED)
            .and_then(|key| key.get_value(APP_ID))
            .map_err(io::Error::from)
        {
            // Windows uses 2/6 for enabled startup entries and 3/7 for disabled entries.
            Ok(value) if value.len() >= 12 => Ok(if matches!(value[0], 2 | 6) {
                Status::Enabled
            } else {
                Status::SystemDisabled
            }),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Windows startup approval value",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Status::Enabled),
            Err(error) => Err(error),
        }
    }

    /// Changes only rtools' user registration and explicitly re-enables its Task Manager override.
    pub(crate) fn set_enabled(&self, enabled: bool) -> io::Result<()> {
        if enabled {
            if !self.executable.is_absolute() || !self.executable.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "startup executable must be an existing absolute file",
                ));
            }
            let command = self.command()?;
            CURRENT_USER.create(RUN)?.set_string(APP_ID, command)?;
            CURRENT_USER.create(APPROVED)?.set_bytes(
                APP_ID,
                Type::Bytes,
                &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            )?;
            Ok(())
        } else {
            match CURRENT_USER
                .options()
                .write()
                .open(RUN)
                .and_then(|key| key.remove_value(APP_ID))
                .map_err(io::Error::from)
            {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            }
        }
    }

    /// Quotes the executable for CreateProcess and validates the documented Run value limit.
    fn command(&self) -> io::Result<String> {
        let path = self.executable.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "startup path is not valid Unicode",
            )
        })?;
        if path.contains('"') || path.chars().any(char::is_control) || path.ends_with('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Windows startup executable path",
            ));
        }
        let command = format!("\"{path}\" {}", ShellCommand::Background);
        if command.encode_utf16().count() > 260 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows startup command exceeds 260 characters; install rtools at a shorter path",
            ));
        }
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Executable paths with spaces remain a single argument, with no console shell involved.
    #[test]
    fn quotes_the_executable_and_rejects_invalid_run_values() {
        let startup = Autostart {
            executable: PathBuf::from(r"C:\Program Files\中文工具\rtools.exe"),
        };
        assert_eq!(
            startup.command().unwrap(),
            "\"C:\\Program Files\\中文工具\\rtools.exe\" background"
        );
        for path in ["a\"b.exe".to_owned(), "a".repeat(261)] {
            assert!(
                Autostart {
                    executable: PathBuf::from(path)
                }
                .command()
                .is_err()
            );
        }
    }
}
