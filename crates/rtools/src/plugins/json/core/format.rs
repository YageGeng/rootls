//! Reading JSON text, pretty-printing it and the toolbar's in-place rewrites.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use rust_i18n::t;
use serde::Serialize;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use serde_json::ser::{PrettyFormatter, Serializer};

use super::JsonError;

/// Indent width used for every pretty-printed output, JSON and XML alike.
pub(crate) const INDENT: usize = 4;

/// JSON source text. Reading it skips a leading BOM and rejects objects that
/// repeat a key: results are written back over the source, so silently
/// keeping one of the values would lose data.
pub struct Json<'a>(pub &'a str);

impl Json<'_> {
    /// Checks that the text is one JSON value with distinct object keys,
    /// without building it; blank text is an empty document and passes.
    pub fn validate(&self) -> Result<(), JsonError> {
        let src = strip_bom(self.0);
        if !src.trim().is_empty() {
            serde_json::from_str::<UniqueKeys>(src)?;
        }
        Ok(())
    }
}

impl TryFrom<Json<'_>> for Value {
    type Error = JsonError;

    /// Parses the text, keeping key order and number spelling.
    fn try_from(Json(src): Json<'_>) -> Result<Self, JsonError> {
        let src = strip_bom(src);
        serde_json::from_str::<UniqueKeys>(src)?;
        Ok(serde_json::from_str(src)?)
    }
}

/// Displays a JSON value pretty-printed with the shared indent.
pub struct Pretty<'a>(pub &'a Value);

impl fmt::Display for Pretty<'_> {
    /// Writes the indented JSON text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let indent = [b' '; INDENT];
        let mut out = Vec::new();
        let mut serializer =
            Serializer::with_formatter(&mut out, PrettyFormatter::with_indent(&indent));
        self.0.serialize(&mut serializer).map_err(|_| fmt::Error)?;
        f.write_str(std::str::from_utf8(&out).map_err(|_| fmt::Error)?)
    }
}

/// In-place rewrites of the source offered by the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rewrite {
    /// Pretty-print with the shared indent.
    Format,
    /// Drop insignificant whitespace.
    Minify,
    /// Minify, then escape as JSON string content without the outer quotes.
    Escape,
    /// Reverse `Escape`: quoted or bare escaped text, pretty-printed when it is JSON.
    Unescape,
}

impl Rewrite {
    /// Applies the rewrite to `src`.
    pub fn apply(self, src: &str) -> Result<String, JsonError> {
        match self {
            Self::Format => Ok(Pretty(&Value::try_from(Json(src))?).to_string()),
            Self::Minify => Ok(Value::try_from(Json(src))?.to_string()),
            Self::Escape => {
                let quoted = Value::String(Self::Minify.apply(src)?).to_string();
                Ok(quoted[1..quoted.len() - 1].to_string())
            }
            Self::Unescape => {
                let trimmed = src.trim();
                let is_quoted =
                    trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"');
                // Bare escaped text becomes a JSON string literal once wrapped in quotes.
                let literal = if is_quoted {
                    Cow::Borrowed(trimmed)
                } else {
                    Cow::Owned(format!("\"{trimmed}\""))
                };
                let text: String = serde_json::from_str(&literal).map_err(|err| {
                    let mut error = JsonError::from(err);
                    // The added opening quote shifts first-line columns by one;
                    // report them against the text the user typed.
                    if !is_quoted
                        && let JsonError::Parse {
                            line: 1, column, ..
                        } = &mut error
                    {
                        *column = column.saturating_sub(1);
                    }
                    error
                })?;
                Ok(Self::Format.apply(&text).unwrap_or(text))
            }
        }
    }
}

/// Drops a leading UTF-8 byte order mark: some editors prepend one, it is not JSON,
/// and `trim` does not treat it as whitespace.
pub(crate) fn strip_bom(input: &str) -> &str {
    input.strip_prefix('\u{feff}').unwrap_or(input)
}

/// Marker produced by a validation-only pass that checks every object has
/// distinct keys; serde_json attaches the line and column to its error.
struct UniqueKeys;

impl<'de> Deserialize<'de> for UniqueKeys {
    /// Walks any JSON value without building it.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueKeys)
    }
}

impl<'de> Visitor<'de> for UniqueKeys {
    type Value = UniqueKeys;

    /// Describes the accepted input for serde error messages.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    /// Accepts `true` / `false`.
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self, E> {
        Ok(self)
    }

    /// Accepts negative integers.
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self, E> {
        Ok(self)
    }

    /// Accepts non-negative integers.
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self, E> {
        Ok(self)
    }

    /// Accepts floating-point numbers.
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self, E> {
        Ok(self)
    }

    /// Accepts strings (and arbitrary-precision number text).
    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self, E> {
        Ok(self)
    }

    /// Accepts `null`.
    fn visit_unit<E: de::Error>(self) -> Result<Self, E> {
        Ok(self)
    }

    /// Checks every array element.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self, A::Error> {
        while seq.next_element::<UniqueKeys>()?.is_some() {}
        Ok(self)
    }

    /// Fails on the first repeated key, then checks every member value.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self, A::Error> {
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            // `replace` hands back the earlier, equal key, so no clone is needed.
            if let Some(key) = seen.replace(key) {
                return Err(de::Error::custom(t!("json.error.duplicate_key", key = key)));
            }
            map.next_value::<UniqueKeys>()?;
        }
        Ok(self)
    }
}
