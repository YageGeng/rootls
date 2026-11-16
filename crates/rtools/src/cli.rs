//! Commands accepted on the command line and over the single-instance socket.

use std::fmt;
use std::str::FromStr;

/// What a second `rtools` process asks the running instance to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellCommand {
    /// Start the resident process without opening or changing any window.
    Background,
    /// Show the launcher, or hide it when it is visible.
    Toggle,
    /// Open the settings window, or focus it when it is already open.
    Settings,
    /// Exit the running instance.
    Quit,
}

/// A command word that names no [`ShellCommand`].
#[derive(Debug, thiserror::Error)]
#[error("unknown command: {0} (expected background, toggle, settings or quit)")]
pub struct UnknownCommand(String);

impl FromStr for ShellCommand {
    type Err = UnknownCommand;

    /// Parses shell commands, ignoring surrounding whitespace and ASCII case.
    fn from_str(text: &str) -> Result<Self, UnknownCommand> {
        match text.trim().to_ascii_lowercase().as_str() {
            "background" => Ok(Self::Background),
            "toggle" => Ok(Self::Toggle),
            "settings" => Ok(Self::Settings),
            "quit" => Ok(Self::Quit),
            _ => Err(UnknownCommand(text.trim().to_string())),
        }
    }
}

impl fmt::Display for ShellCommand {
    /// Writes the wire / command-line spelling.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Background => "background",
            Self::Toggle => "toggle",
            Self::Settings => "settings",
            Self::Quit => "quit",
        })
    }
}
