//! Legacy LaunchAgent registration with launchd overrides and macOS background-item consent.

use std::{io, os::unix::fs::PermissionsExt as _, process::Command};

use objc2_foundation::NSURL;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use super::{Status, file::RegistrationFile};
use crate::shell::APP_ID;

/// Owns the registration file and the current user's GUI launchd domain.
pub(crate) struct Autostart {
    registration: RegistrationFile,
    domain: String,
}

impl Autostart {
    /// Resolves the current user's registration without changing native startup state.
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: geteuid takes no pointers and has no preconditions.
        let uid = unsafe { libc::geteuid() };
        Ok(Self {
            registration: RegistrationFile::new()?,
            domain: format!("gui/{uid}"),
        })
    }

    /// Requires a valid file, no launchd disable override, and native background-item approval.
    pub(crate) fn status(&self) -> io::Result<Status> {
        if !self.registration.is_enabled()? {
            return Ok(Status::Disabled);
        }
        if self.registration.path.metadata()?.permissions().mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "startup plist is group/world-writable; turn startup off and on to repair it",
            ));
        }
        let output = Command::new("/bin/launchctl")
            .args(["print-disabled", &self.domain])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "launchctl print-disabled failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let text = std::str::from_utf8(&output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if launchd_disabled(text)? {
            return Ok(Status::SystemDisabled);
        }
        // SMAppService is unavailable before macOS 13; legacy launchd remains supported there.
        if objc2::runtime::AnyClass::get(c"SMAppService").is_none() {
            return Ok(Status::Enabled);
        }
        objc2::rc::autoreleasepool(|_| {
            let url = NSURL::from_path(&self.registration.path, false, None).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid startup plist URL")
            })?;
            // SAFETY: The class is available and the retained file URL remains alive through the call.
            Status::try_from(unsafe { SMAppService::statusForLegacyURL(&url) })
        })
    }

    /// Clears launchd's persistent disable override only on an explicit enable request.
    pub(crate) fn set_enabled(&self, enabled: bool) -> io::Result<()> {
        self.registration.set_enabled(enabled)?;
        if enabled {
            let output = Command::new("/bin/launchctl")
                .args(["enable", &format!("{}/{APP_ID}", self.domain)])
                .output()?;
            if !output.status.success() {
                return Err(io::Error::other(format!(
                    "launchctl enable failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }
        Ok(())
    }
}

impl TryFrom<SMAppServiceStatus> for Status {
    type Error = io::Error;

    /// An on-disk legacy plist may not be known to ServiceManagement until the next login.
    fn try_from(status: SMAppServiceStatus) -> io::Result<Self> {
        match status {
            SMAppServiceStatus::Enabled => Ok(Self::Enabled),
            SMAppServiceStatus::RequiresApproval => Ok(Self::RequiresApproval),
            SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound => Ok(Self::Pending),
            other => Err(io::Error::other(format!(
                "unknown background-item status: {}",
                other.0
            ))),
        }
    }
}

/// Reads both boolean and named launchctl formats without mistaking another service for rtools.
fn launchd_disabled(text: &str) -> io::Result<bool> {
    let entries = text
        .trim()
        .strip_prefix("disabled services = {")
        .and_then(|text| text.trim_end().strip_suffix('}'))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "unrecognized launchctl disabled-services output",
            )
        })?;
    for line in entries
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (label, value) = line.split_once("=>").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid launchctl disabled-services entry",
            )
        })?;
        if label.trim().trim_matches('"') == APP_ID {
            return match value.trim() {
                "true" | "disabled" => Ok(true),
                "false" | "enabled" => Ok(false),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unknown launchctl disable override",
                )),
            };
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Explicit disable overrides survive a valid plist and must not be reported as enabled.
    #[test]
    fn reads_old_and_new_launchctl_override_formats() {
        for (value, disabled) in [
            ("true", true),
            ("disabled", true),
            ("false", false),
            ("enabled", false),
        ] {
            let text = format!("disabled services = {{\n\t\"{APP_ID}\" => {value}\n}}\n");
            assert_eq!(launchd_disabled(&text).unwrap(), disabled);
        }
        assert!(!launchd_disabled("disabled services = {\n\"other.app\" => disabled\n}").unwrap());
        assert!(launchd_disabled("disabled services = {").is_err());
        assert!(
            launchd_disabled(&format!(
                "disabled services = {{\n\"{APP_ID}\" => unknown\n}}"
            ))
            .is_err()
        );
    }

    /// Approval and pending registration must remain distinct from runnable startup.
    #[test]
    fn maps_native_approval_without_false_success() {
        for (native, expected) in [
            (SMAppServiceStatus::Enabled, Status::Enabled),
            (
                SMAppServiceStatus::RequiresApproval,
                Status::RequiresApproval,
            ),
            (SMAppServiceStatus::NotRegistered, Status::Pending),
            (SMAppServiceStatus::NotFound, Status::Pending),
        ] {
            let status = Status::try_from(native).unwrap();
            assert_eq!(status, expected);
            assert!(status.registered());
        }
        assert!(Status::try_from(SMAppServiceStatus(99)).is_err());
    }
}
