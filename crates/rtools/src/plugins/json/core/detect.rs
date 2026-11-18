//! Detection of pasted input formats and their normalisation to JSON (spec §5.4).

use std::borrow::Cow;
use std::str::FromStr;

use rust_i18n::t;
use serde_json::{Number, Value};

use super::convert::{query::Query, xml::Xml, yaml::Yaml};
use super::format::strip_bom;
use super::{Json, JsonError, Pretty};

/// Text formats the plugin can turn into JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Json,
    Xml,
    UrlParams,
    Yaml,
}

impl InputKind {
    /// Localized format name shown to the user, e.g. in "Recognized as XML".
    pub fn label(self) -> Cow<'static, str> {
        match self {
            Self::Json => "JSON".into(),
            Self::Xml => "XML".into(),
            Self::UrlParams => t!("json.url_params"),
            Self::Yaml => "YAML".into(),
        }
    }

    /// Guesses a document format by prefix before full validation or conversion.
    pub fn sniff(input: &str) -> Option<InputKind> {
        let text = strip_bom(input).trim();
        match text.chars().next()? {
            // A leading quote is a JSON string or a copied `"key": value` fragment.
            '{' | '[' | '"' => Some(Self::Json),
            _ if matches!(text, "true" | "false" | "null") => Some(Self::Json),
            '-' | '0'..='9' if text.parse::<Number>().is_ok() => Some(Self::Json),
            '<' => Some(Self::Xml),
            // A single token with `=` and no whitespace (hence one line) reads as URL params.
            _ if text.contains('=') && !text.chars().any(char::is_whitespace) => {
                Some(Self::UrlParams)
            }
            _ => None,
        }
    }

    /// Adds multi-line YAML as the last candidate, retaining its indentation and line endings.
    fn candidate(source: &str) -> Option<Self> {
        Self::sniff(source).or_else(|| source.contains(['\n', '\r']).then_some(Self::Yaml))
    }

    /// Returns the format and whether its complete source was verified for launcher ranking.
    pub(crate) fn recognize(input: &str) -> Option<(Self, bool)> {
        let source = strip_bom(input);
        let format = Self::candidate(input)?;
        let verified = match format {
            Self::Json => Json(input).validate().is_ok(),
            Self::Yaml => {
                Yaml(source).validate_document().ok()?;
                true
            }
            Self::Xml | Self::UrlParams => false,
        };
        Some((format, verified))
    }
}

/// Pasted text converted to formatted JSON.
#[derive(Debug, PartialEq)]
pub struct Normalized(pub String);

impl FromStr for Normalized {
    type Err = JsonError;

    /// Detects the input format and converts it to formatted JSON text.
    ///
    /// Text starting with `{`, `[` or `"` is always treated as JSON, so a broken
    /// document reports its parse error instead of being re-read as YAML.
    fn from_str(input: &str) -> Result<Self, JsonError> {
        let source = strip_bom(input);
        let value = match InputKind::candidate(input).ok_or(JsonError::Unrecognized)? {
            // Untrimmed source keeps parse error positions aligned with the user's text.
            InputKind::Json => Value::try_from(Json(input))?,
            InputKind::Xml => Value::try_from(Xml(source.trim()))?,
            InputKind::UrlParams => Value::from(Query(source.trim())),
            // YAML indentation and final newlines carry data, particularly in block scalars.
            InputKind::Yaml => {
                Value::try_from(Yaml(source)).map_err(|_| JsonError::Unrecognized)?
            }
        };
        Ok(Self(Pretty(&value).to_string()))
    }
}
