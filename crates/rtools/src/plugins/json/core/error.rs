//! Error type shared by the JSON core.

use std::fmt;

use rust_i18n::t;

/// Errors surfaced to the plugin status line; `Display` is localized.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonError {
    /// Invalid JSON text, with the 1-based position of the problem.
    Parse {
        line: usize,
        column: usize,
        message: String,
    },
    /// Malformed XML input.
    Xml(String),
    /// Malformed YAML input, or YAML whose top level is not a mapping or sequence.
    Yaml(String),
    /// Input that matches none of the supported formats.
    Unrecognized,
    /// A JavaScript exception raised by a filter expression or document parse.
    Script(String),
    /// A filter expression exceeded its time budget.
    Timeout,
    /// The filter worker thread is gone.
    EngineStopped,
}

impl fmt::Display for JsonError {
    /// Writes the message in the current interface language.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Parse {
                line,
                column,
                message,
            } => t!(
                "json.error.parse",
                line = line,
                column = column,
                message = message
            ),
            Self::Xml(message) => t!("json.error.xml", message = message),
            Self::Yaml(message) => t!("json.error.yaml", message = message),
            Self::Unrecognized => t!("json.error.unrecognized"),
            Self::Script(message) => t!("json.error.script", message = message),
            Self::Timeout => t!("json.error.timeout"),
            Self::EngineStopped => t!("json.error.engine_stopped"),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for JsonError {}

impl JsonError {
    /// Short status-line text: parse errors lead with their position.
    pub fn brief(&self) -> String {
        match self {
            JsonError::Parse {
                line,
                column,
                message,
            } => t!(
                "json.error.position",
                line = line,
                column = column,
                message = message
            )
            .into_owned(),
            other => other.to_string(),
        }
    }
}

impl From<serde_json::Error> for JsonError {
    /// Converts a serde_json error, moving its position into dedicated fields.
    fn from(err: serde_json::Error) -> Self {
        let text = err.to_string();
        // serde_json appends " at line N column M"; the position is rendered separately.
        let message = match text.rfind(" at line ") {
            Some(idx) if err.line() > 0 => text[..idx].to_string(),
            _ => text,
        };
        JsonError::Parse {
            line: err.line(),
            column: err.column(),
            message,
        }
    }
}

impl From<quick_xml::Error> for JsonError {
    /// Wraps a quick-xml reader error.
    fn from(err: quick_xml::Error) -> Self {
        JsonError::Xml(err.to_string())
    }
}

impl From<quick_xml::events::attributes::AttrError> for JsonError {
    /// Wraps a malformed-attribute error.
    fn from(err: quick_xml::events::attributes::AttrError) -> Self {
        JsonError::Xml(err.to_string())
    }
}

impl From<rquickjs::Error> for JsonError {
    /// Wraps a QuickJS error that carries no pending JavaScript exception.
    fn from(err: rquickjs::Error) -> Self {
        JsonError::Script(err.to_string())
    }
}

impl From<futures::channel::oneshot::Canceled> for JsonError {
    /// A dropped reply channel means the worker thread has stopped.
    fn from(_: futures::channel::oneshot::Canceled) -> Self {
        JsonError::EngineStopped
    }
}
