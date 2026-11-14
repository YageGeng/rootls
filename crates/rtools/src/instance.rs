//! Single-instance coordination over a local socket (spec §4.1).

use std::io::{self, BufRead, BufReader, Write};
use std::time::Duration;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream};

use crate::cli::ShellCommand;

/// How long a client may take to send its command line.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// Consecutive `accept` failures after which the listener gives up.
const MAX_ACCEPT_FAILURES: u32 = 8;

/// The socket shared by every `rtools` process of one user: a socket file path
/// on Linux (abstract names are reachable by every local user), a namespaced
/// name elsewhere.
pub struct InstanceName(pub String);

impl InstanceName {
    /// `$XDG_RUNTIME_DIR/rtools.sock`, falling back to a private `0700`
    /// directory in the temp dir when the session has no runtime dir.
    #[cfg(target_os = "linux")]
    pub fn for_current_user() -> Self {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        let dir = dirs::runtime_dir().unwrap_or_else(|| {
            let dir = std::env::temp_dir().join(format!("rtools-{}", user_tag()));
            let created = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir);
            // An existing directory keeps its mode; tighten it in case it was created loosely.
            if let Err(err) = created.and_then(|()| {
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            }) {
                tracing::warn!(
                    "preparing socket directory {} failed: {}",
                    dir.display(),
                    err
                );
            }
            dir
        });
        Self(dir.join("rtools.sock").display().to_string())
    }

    /// `rtools-<user>.sock`, so different users on one machine never collide.
    #[cfg(not(target_os = "linux"))]
    pub fn for_current_user() -> Self {
        Self(format!("rtools-{}.sock", user_tag()))
    }

    /// Connects to a running instance and writes one command line.
    /// Fails fast when no instance is listening.
    pub fn send(&self, command: ShellCommand) -> io::Result<()> {
        let mut stream = Stream::connect(self.socket_name()?)?;
        stream.write_all(format!("{command}\n").as_bytes())?;
        stream.flush()
    }

    /// Starts listening. A socket that still answers belongs to a live instance
    /// and is left alone; one left by a crashed instance is replaced.
    pub fn bind(&self) -> io::Result<InstanceServer> {
        let listen = |overwrite| {
            ListenerOptions::new()
                .name(self.socket_name()?)
                .try_overwrite(overwrite)
                .create_sync()
        };
        let listener = match listen(false) {
            Err(err)
                if err.kind() == io::ErrorKind::AddrInUse
                    && Stream::connect(self.socket_name()?).is_err() =>
            {
                listen(true)?
            }
            result => result?,
        };
        Ok(InstanceServer { listener })
    }

    /// The platform socket name for this instance name.
    #[cfg(target_os = "linux")]
    fn socket_name(&self) -> io::Result<Name<'_>> {
        self.0
            .as_str()
            .to_fs_name::<interprocess::local_socket::GenericFilePath>()
    }

    /// The platform socket name for this instance name.
    #[cfg(not(target_os = "linux"))]
    fn socket_name(&self) -> io::Result<Name<'_>> {
        self.0
            .as_str()
            .to_ns_name::<interprocess::local_socket::GenericNamespaced>()
    }
}

/// The current user's name reduced to ASCII alphanumerics.
fn user_tag() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "default".into())
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// The running instance's listener.
pub struct InstanceServer {
    listener: Listener,
}

impl InstanceServer {
    /// Blocking iterator: one parsed command (or client error) per accepted
    /// connection. Ends after [`MAX_ACCEPT_FAILURES`] consecutive `accept`
    /// failures, since the listener is then unusable.
    pub fn incoming(self) -> impl Iterator<Item = io::Result<ShellCommand>> {
        let listener = self.listener;
        let mut failures = 0;
        std::iter::from_fn(move || {
            loop {
                match listener.accept() {
                    Ok(stream) => {
                        failures = 0;
                        // A connection closed without a line is another process's
                        // liveness probe (see `bind`), not a command.
                        if let Some(command) = read_command(stream).transpose() {
                            return Some(command);
                        }
                    }
                    Err(err) if failures + 1 < MAX_ACCEPT_FAILURES => {
                        failures += 1;
                        tracing::warn!("accepting an instance connection failed: {}", err);
                    }
                    Err(err) => {
                        tracing::error!(
                            "instance socket keeps failing ({}); no longer accepting commands",
                            err
                        );
                        return None;
                    }
                }
            }
        })
    }
}

/// Reads one command line from a client connection within [`READ_TIMEOUT`];
/// `None` when the client closed without sending anything.
fn read_command(stream: Stream) -> io::Result<Option<ShellCommand>> {
    stream.set_recv_timeout(Some(READ_TIMEOUT))?;
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line)? == 0 {
        return Ok(None);
    }
    line.parse()
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}
