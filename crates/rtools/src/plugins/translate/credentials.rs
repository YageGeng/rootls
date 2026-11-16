//! Platform credentials are isolated per endpoint and never serialized with preferences.
use gpui_kit::App;

use super::core::{Endpoint, TranslateError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Saved,
    Environment,
}

impl KeySource {
    /// Returns an honest source label without revealing any part of the key.
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "translate.key_saved_source",
            Self::Environment => "translate.key_environment_source",
        }
    }
}

/// Secret material has no Debug, Display or serialization implementation.
pub struct Credential {
    pub source: KeySource,
    pub storage_failed: bool,
    pub(crate) secret: String,
}

impl Credential {
    /// Resolves saved-before-environment precedence without retrying with another key.
    pub fn resolve(
        saved: Option<Vec<u8>>,
        environment: Option<String>,
    ) -> Result<Self, TranslateError> {
        let (secret, source) = match saved {
            Some(saved) => (
                String::from_utf8(saved).map_err(|_| TranslateError::Credentials)?,
                KeySource::Saved,
            ),
            None => (
                environment.ok_or(TranslateError::MissingKey)?,
                KeySource::Environment,
            ),
        };
        let secret = secret.trim().to_string();
        if secret.is_empty() {
            return Err(TranslateError::MissingKey);
        }
        if !secret.is_ascii() || secret.chars().any(char::is_control) {
            return Err(TranslateError::Credentials);
        }
        Ok(Self {
            secret,
            source,
            storage_failed: false,
        })
    }
}

impl Endpoint {
    /// Reads the platform store asynchronously, falling back to this process's environment.
    pub fn load_key(
        &self,
        cx: &App,
    ) -> impl Future<Output = Result<Credential, TranslateError>> + Send + 'static {
        let read = cx.read_credentials(&self.credential_id());
        let environment = std::env::var("DEEPSEEK_API_KEY").ok();
        async move {
            match read.await {
                Ok(saved) => Credential::resolve(saved.map(|(_, secret)| secret), environment),
                Err(_) => {
                    tracing::warn!(
                        "translation credential storage is unavailable; checking process environment"
                    );
                    let mut key = Credential::resolve(None, environment)
                        .map_err(|_| TranslateError::Credentials)?;
                    key.storage_failed = true;
                    Ok(key)
                }
            }
        }
    }
}
