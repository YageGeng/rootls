//! LaunchAgent and XDG registrations, written atomically for the current user.

use std::{
    fs, io,
    io::Write as _,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use crate::{cli::ShellCommand, shell::APP_ID};

/// A registration at a stable user-owned path, pointing at the installed executable.
pub(crate) struct RegistrationFile {
    pub(super) path: PathBuf,
    executable: PathBuf,
    format: Format,
}

/// The two declarative login-entry formats used by Unix desktops.
#[derive(Clone, Copy)]
enum Format {
    LaunchAgent,
    Desktop,
}

impl RegistrationFile {
    /// Resolves user directories without creating files or enabling login startup.
    pub(crate) fn new() -> io::Result<Self> {
        let (directory, format, extension) = if cfg!(target_os = "macos") {
            (
                dirs::home_dir().map(|home| home.join("Library/LaunchAgents")),
                Format::LaunchAgent,
                "plist",
            )
        } else {
            (
                dirs::config_dir().map(|config| config.join("autostart")),
                Format::Desktop,
                "desktop",
            )
        };
        let directory = directory.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "user startup directory is unavailable",
            )
        })?;
        Ok(Self {
            path: directory.join(format!("{APP_ID}.{extension}")),
            executable: std::env::current_exe()?,
            format,
        })
    }

    /// Reads the native registration rather than a potentially stale TOML preference.
    pub(crate) fn is_enabled(&self) -> io::Result<bool> {
        match fs::read(&self.path) {
            Ok(text) => self.format.enabled_bytes(&text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// XDG startup has no separate authorization service; the desktop entry defines its state.
    #[cfg(target_os = "linux")]
    pub(crate) fn status(&self) -> io::Result<super::Status> {
        self.is_enabled().map(|enabled| {
            if enabled {
                super::Status::Enabled
            } else {
                super::Status::Disabled
            }
        })
    }

    /// Changes the next login's startup behavior without launching or quitting the running app.
    pub(crate) fn set_enabled(&self, enabled: bool) -> io::Result<()> {
        if !enabled && matches!(self.format, Format::LaunchAgent) {
            return match fs::remove_file(&self.path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        }
        if enabled && (!self.executable.is_absolute() || !self.executable.is_file()) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "startup executable must be an existing absolute file",
            ));
        }
        let text = self.format.render(&self.executable, enabled)?;
        let directory = self.path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "startup entry has no parent directory",
            )
        })?;
        fs::create_dir_all(directory)?;
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        // Private, distinct files prevent permissive umasks and concurrent writers from corrupting a registration.
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temp.write_all(text.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }
}

impl Format {
    /// Encodes the executable directly, without invoking a shell or inheriting its PATH.
    fn render(self, executable: &Path, enabled: bool) -> io::Result<String> {
        let path = executable.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "startup path is not valid Unicode",
            )
        })?;
        if path.chars().any(char::is_control) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "startup path contains control characters",
            ));
        }
        let command = ShellCommand::Background;
        match self {
            Self::LaunchAgent => {
                let path = quick_xml::escape::escape(path);
                Ok(format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                    <plist version=\"1.0\"><dict>\n\
                    <key>Label</key><string>{APP_ID}</string>\n\
                    <key>AssociatedBundleIdentifiers</key><array><string>{APP_ID}</string></array>\n\
                    <key>ProgramArguments</key><array><string>{path}</string><string>{command}</string></array>\n\
                    <key>LimitLoadToSessionType</key><string>Aqua</string>\n\
                    <key>RunAtLoad</key><true/>\n</dict></plist>\n"
                ))
            }
            Self::Desktop => {
                if path.contains('=') {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "XDG startup executable paths cannot contain '='",
                    ));
                }
                let mut quoted = String::from("\"");
                for character in path.chars() {
                    // Desktop strings unescape once before Exec quoting is interpreted.
                    match character {
                        '\\' => quoted.push_str(r"\\\\"),
                        '"' | '`' | '$' => {
                            quoted.push_str(r"\\");
                            quoted.push(character);
                        }
                        '%' => quoted.push_str("%%"),
                        other => quoted.push(other),
                    }
                }
                quoted.push('"');
                Ok(format!(
                    "[Desktop Entry]\nType=Application\nName=rtools\nExec={quoted} {command}\nIcon={APP_ID}\nTerminal=false\nStartupNotify=false\nHidden={}\n",
                    !enabled
                ))
            }
        }
    }

    /// Honors native disable flags; a malformed entry is an error rather than a false success.
    fn enabled_bytes(self, bytes: &[u8]) -> io::Result<bool> {
        match self {
            Self::LaunchAgent => {
                let value = plist::Value::from_reader(io::Cursor::new(bytes))
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                let dictionary = value.as_dictionary().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "startup plist root is not a dictionary",
                    )
                })?;
                let run_at_load = dictionary
                    .get("RunAtLoad")
                    .and_then(plist::Value::as_boolean)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "startup plist is missing a boolean RunAtLoad",
                        )
                    })?;
                let disabled = match dictionary.get("Disabled") {
                    None => false,
                    Some(value) => value.as_boolean().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "startup plist Disabled is not a boolean",
                        )
                    })?,
                };
                Ok(run_at_load && !disabled)
            }
            Self::Desktop => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                let (mut in_entry, mut application, mut executable, mut hidden, mut gnome_enabled) =
                    (false, false, false, false, true);
                for line in text.lines().map(str::trim) {
                    if line.starts_with('[') {
                        in_entry = line == "[Desktop Entry]";
                    } else if in_entry && let Some((key, value)) = line.split_once('=') {
                        match key.trim() {
                            "Type" => application = value.trim() == "Application",
                            "Exec" => executable = !value.trim().is_empty(),
                            "Hidden" => hidden = value.trim() == "true",
                            "X-GNOME-Autostart-enabled" => gnome_enabled = value.trim() != "false",
                            _ => {}
                        }
                    }
                }
                if !application || !executable {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "startup desktop entry is missing Type or Exec",
                    ));
                }
                Ok(!hidden && gnome_enabled)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both file backends can be toggled repeatedly without touching the real login settings.
    #[test]
    fn registrations_round_trip_in_an_isolated_directory() {
        for format in [Format::LaunchAgent, Format::Desktop] {
            let directory = tempfile::tempdir().unwrap();
            let executable = directory.path().join("rtools with spaces & 中文");
            fs::write(&executable, "test executable").unwrap();
            let startup = RegistrationFile {
                path: directory.path().join("autostart/entry"),
                executable,
                format,
            };
            assert!(!startup.is_enabled().unwrap());
            startup.set_enabled(true).unwrap();
            assert!(startup.is_enabled().unwrap());
            startup.set_enabled(true).unwrap();
            startup.set_enabled(false).unwrap();
            startup.set_enabled(false).unwrap();
            assert!(!startup.is_enabled().unwrap());
            if matches!(format, Format::Desktop) {
                // A user override also suppresses a same-named system entry.
                assert!(
                    fs::read_to_string(startup.path)
                        .unwrap()
                        .contains("Hidden=true")
                );
            }
        }
    }

    /// Paths survive both desktop-string and Exec quoting without becoming field codes.
    #[test]
    fn desktop_exec_escapes_reserved_characters() {
        let text = Format::Desktop
            .render(Path::new("/opt/a b/\"$`\\%f/rtools"), true)
            .unwrap();
        assert!(
            text.contains(r#"Exec="/opt/a b/\\"\\$\\`\\\\%%f/rtools" background"#),
            "{text}"
        );
        assert!(
            Format::Desktop
                .render(Path::new("/opt/a=b/rtools"), true)
                .is_err()
        );
        assert!(
            Format::Desktop
                .render(Path::new("/opt/a\nb/rtools"), true)
                .is_err()
        );
    }

    /// GIO must launch the exact escaped path and pass the background command unchanged.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires the GIO desktop launcher command"]
    fn desktop_entry_launches_with_gio() {
        use std::os::unix::fs::PermissionsExt as _;
        use std::{process::Command, thread, time::Duration};

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("rtools 空格 &\"$`\\%f");
        fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s' \"$1\" > \"$(dirname \"$0\")/result\"\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let entry = directory.path().join("rtools.desktop");
        fs::write(&entry, Format::Desktop.render(&executable, true).unwrap()).unwrap();
        let output = Command::new("gio")
            .arg("launch")
            .arg(entry)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = directory.path().join("result");
        for _ in 0..100 {
            if fs::read_to_string(&result).is_ok_and(|text| text == "background") {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("desktop entry did not launch the expected executable with the background argument");
    }

    /// Plists escape paths as XML and keep background startup independent of input shell syntax.
    #[test]
    fn launch_agent_escapes_xml_and_does_not_restart_after_quit() {
        let text = Format::LaunchAgent
            .render(Path::new("/Applications/A&B <tools>.app/rtools"), true)
            .unwrap();
        assert!(text.contains("A&amp;B &lt;tools&gt;.app/rtools"));
        assert!(text.contains("<string>background</string>"));
        assert!(text.contains("<key>RunAtLoad</key><true/>"));
        assert!(!text.contains("KeepAlive"));
        assert!(Format::LaunchAgent.enabled_bytes(text.as_bytes()).unwrap());
        assert!(
            !Format::LaunchAgent
                .enabled_bytes(text.replace("<true/>", "<false/>").as_bytes())
                .unwrap()
        );
        assert!(Format::LaunchAgent.enabled_bytes(b"not a plist").is_err());
    }

    /// Apple's parser must recover the exact executable and background argument from our plist.
    #[cfg(target_os = "macos")]
    #[test]
    fn launch_agent_is_accepted_by_plutil() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("startup.plist");
        let executable = "/Applications/A&B <tools> 中文.app/Contents/MacOS/rtools";
        fs::write(
            &path,
            Format::LaunchAgent
                .render(Path::new(executable), true)
                .unwrap(),
        )
        .unwrap();
        let output = std::process::Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-", "--"])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let plist: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            plist["ProgramArguments"],
            serde_json::json!([executable, "background"])
        );
        assert_eq!(plist["RunAtLoad"], true);
        assert_eq!(plist["LimitLoadToSessionType"], "Aqua");
        assert!(plist.get("KeepAlive").is_none());
    }

    /// Desktop managers can disable the entry without removing it.
    #[test]
    fn desktop_status_respects_external_disable_flags_and_sections() {
        for flag in ["Hidden=true", "X-GNOME-Autostart-enabled=false"] {
            assert!(
                !Format::Desktop
                    .enabled_bytes(
                        format!(
                            "[Desktop Entry]\nType=Application\nExec=rtools background\n{flag}\n"
                        )
                        .as_bytes()
                    )
                    .unwrap()
            );
        }
        assert!(Format::Desktop.enabled_bytes(b"[Desktop Entry]\nType=Application\nExec=rtools background\n[Desktop Action Other]\nHidden=true\n").unwrap());
        assert!(
            Format::Desktop
                .enabled_bytes(b"not a desktop entry")
                .is_err()
        );
    }

    /// A failed enable never replaces the previous valid registration.
    #[test]
    fn invalid_executable_keeps_the_existing_registration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("entry");
        fs::write(&path, "original").unwrap();
        let startup = RegistrationFile {
            path: path.clone(),
            executable: directory.path().join("missing"),
            format: Format::Desktop,
        };
        assert!(startup.set_enabled(true).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "original");
    }

    /// Truncated XML must never be reported as an enabled login registration.
    #[test]
    fn truncated_plist_is_rejected() {
        let text = "<plist version=\"1.0\"><dict><key>RunAtLoad</key><true/>";
        assert!(Format::LaunchAgent.enabled_bytes(text.as_bytes()).is_err());
    }

    /// Native tools may convert the entry to binary without changing its startup semantics.
    #[test]
    fn binary_plists_preserve_disable_status() {
        let text = Format::LaunchAgent
            .render(Path::new("/Applications/rtools.app/rtools"), true)
            .unwrap();
        let mut value = plist::Value::from_reader(io::Cursor::new(text)).unwrap();
        let mut bytes = Vec::new();
        value.to_writer_binary(&mut bytes).unwrap();
        assert!(Format::LaunchAgent.enabled_bytes(&bytes).unwrap());
        value
            .as_dictionary_mut()
            .unwrap()
            .insert("Disabled".into(), plist::Value::Boolean(true));
        bytes.clear();
        value.to_writer_binary(&mut bytes).unwrap();
        assert!(!Format::LaunchAgent.enabled_bytes(&bytes).unwrap());
        value
            .as_dictionary_mut()
            .unwrap()
            .insert("Disabled".into(), plist::Value::String("true".into()));
        bytes.clear();
        value.to_writer_binary(&mut bytes).unwrap();
        assert!(Format::LaunchAgent.enabled_bytes(&bytes).is_err());
    }

    /// Replacing a permissive temporary file must not inherit its unsafe mode.
    #[test]
    fn launch_agent_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("rtools");
        fs::write(&executable, "executable").unwrap();
        let path = directory.path().join("entry.plist");
        fs::write(path.with_extension("tmp"), "old temporary file").unwrap();
        fs::set_permissions(
            path.with_extension("tmp"),
            fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        let startup = RegistrationFile {
            path,
            executable,
            format: Format::LaunchAgent,
        };
        startup.set_enabled(true).unwrap();
        assert_eq!(
            fs::metadata(&startup.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// Independent writers must each publish a complete entry without sharing a temporary path.
    #[test]
    fn concurrent_writes_keep_complete_entries() {
        use std::sync::Barrier;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("rtools");
        fs::write(&executable, "executable").unwrap();
        let startup = RegistrationFile {
            path: directory.path().join("entry.plist"),
            executable,
            format: Format::LaunchAgent,
        };
        let gate = Barrier::new(2);
        let failures = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..2)
                .map(|_| {
                    let (startup, gate) = (&startup, &gate);
                    scope.spawn(move || {
                        let mut failures = 0;
                        for _ in 0..20 {
                            gate.wait();
                            failures += usize::from(startup.set_enabled(true).is_err());
                        }
                        failures
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .sum::<usize>()
        });
        assert_eq!(failures, 0);
        assert!(startup.is_enabled().unwrap());
    }
}
