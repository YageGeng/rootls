//! User-level login registration; the operating system is the source of truth.
//! No registration is created merely by starting the application.

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod file;
#[cfg(target_os = "linux")]
pub(crate) use file::RegistrationFile as Autostart;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::Autostart;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::Autostart;

use gpui_kit::{App, AppContext as _, Global};

/// Registration and OS approval are independent; a registered entry may not be runnable yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Disabled,
    Enabled,
    #[cfg(target_os = "macos")]
    Pending,
    #[cfg(target_os = "macos")]
    RequiresApproval,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    SystemDisabled,
}

impl Status {
    /// Keeps the switch on for registered entries so users can still remove blocked registrations.
    pub(crate) fn registered(self) -> bool {
        self != Self::Disabled
    }

    /// Explains native restrictions separately from the user's registration choice.
    pub(crate) fn notice_key(self) -> Option<&'static str> {
        match self {
            Self::Disabled | Self::Enabled => None,
            #[cfg(target_os = "macos")]
            Self::Pending => Some("settings.autostart_pending"),
            #[cfg(target_os = "macos")]
            Self::RequiresApproval => Some("settings.autostart_approval"),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Self::SystemDisabled => Some("settings.autostart_disabled"),
        }
    }
}

/// Serializes native operations for the application, including across settings-window lifetimes.
#[derive(Default)]
pub(crate) struct AutostartStore {
    pub(crate) status: Option<Status>,
    pub(crate) busy: bool,
    pub(crate) error: Option<String>,
}

impl Global for AutostartStore {}

impl AutostartStore {
    /// Claims the operation before spawning work so reopening a window cannot start another write.
    fn begin(&mut self) -> bool {
        if self.busy {
            return false;
        }
        self.busy = true;
        self.error = None;
        true
    }

    /// Runs a read or explicit change off the UI thread and publishes its result application-wide.
    pub(crate) fn request(enabled: Option<bool>, cx: &mut App) {
        if !cx.global_mut::<Self>().begin() {
            return;
        }
        let work = cx.background_spawn(async move { Self::load(enabled) });
        cx.spawn(async move |cx| {
            let result = work.await;
            cx.update(|cx| cx.set_global(result));
        })
        .detach();
    }

    /// Re-reads after failed writes because native operations can partially succeed.
    fn load(enabled: Option<bool>) -> Self {
        let mut result = Self::default();
        let operation = (|| {
            let startup = Autostart::new()?;
            let changed = enabled
                .map(|enabled| {
                    tracing::info!("changing login startup registration to {}", enabled);
                    startup.set_enabled(enabled)
                })
                .transpose();
            let status = startup.status();
            result.status = status.as_ref().ok().copied();
            changed?;
            let status = status?;
            if enabled.is_some_and(|wanted| wanted != status.registered()) {
                return Err(std::io::Error::other(
                    "system startup registration did not accept the change",
                ));
            }
            Ok(())
        })();
        if let Err(error) = operation {
            tracing::warn!(
                "reading or updating login startup registration failed: {}",
                error
            );
            result.error = Some(error.to_string());
        } else if enabled.is_some() {
            tracing::info!("login startup status is now {:?}", result.status);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All windows share the same busy guard until an operation publishes its result.
    #[test]
    fn only_one_operation_can_be_in_flight() {
        let mut store = AutostartStore::default();
        assert!(store.begin());
        assert!(!store.begin());
        store = AutostartStore {
            status: Some(Status::Enabled),
            ..Default::default()
        };
        assert!(store.begin());
        assert!(store.status.unwrap().registered());
    }
}
