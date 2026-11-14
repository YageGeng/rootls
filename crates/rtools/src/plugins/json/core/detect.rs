//! Detection of pasted input formats and their normalisation to JSON (spec §5.4).

use std::borrow::Cow;
use std::str::FromStr;

use rust_i18n::t;
use serde_json::Value;

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

    /// Cheap prefix guess for `Plugin::accepts`; never parses and never returns `Yaml`.
    pub fn sniff(input: &str) -> Option<InputKind> {
        let text = strip_bom(input).trim();
        match text.chars().next()? {
            // A leading quote is a JSON string or a copied `"key": value` fragment.
            '{' | '[' | '"' => Some(Self::Json),
            '<' => Some(Self::Xml),
            // A single token with `=` and no whitespace (hence one line) reads as URL params.
            _ if text.contains('=') && !text.chars().any(char::is_whitespace) => {
                Some(Self::UrlParams)
            }
            _ => None,
        }
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
        let text = source.trim();
        let value = match InputKind::sniff(text).unwrap_or(InputKind::Yaml) {
            // Untrimmed source keeps parse error positions aligned with the user's text.
            InputKind::Json => Value::try_from(Json(source))?,
            InputKind::Xml => Value::try_from(Xml(text))?,
            InputKind::UrlParams => Value::from(Query(text)),
            // Only multi-line text is tried as YAML, and a YAML error only means
            // "not YAML": the user may have pasted plain text.
            InputKind::Yaml if text.contains('\n') => {
                Value::try_from(Yaml(text)).map_err(|_| JsonError::Unrecognized)?
            }
            InputKind::Yaml => return Err(JsonError::Unrecognized),
        };
        Ok(Self(Pretty(&value).to_string()))
    }
}
