//! URL query string → JSON conversion.

use serde_json::Value;

use super::Members;

/// URL parameters: bare (`a=1&b=2`), `?`-prefixed or inside a full URL.
pub struct Query<'a>(pub &'a str);

impl From<Query<'_>> for Value {
    /// Converts the parameters into an object of strings; repeated keys become arrays.
    fn from(Query(src): Query<'_>) -> Self {
        // Only a URL prefix (no `=` before the first `?`) is cut away, together with its
        // `#fragment`; bare parameters may carry raw `?` and `#` inside their values.
        let query = match src.split_once('?') {
            Some((prefix, rest)) if !prefix.contains('=') => {
                rest.split_once('#').map_or(rest, |(query, _)| query)
            }
            _ => src,
        };
        form_urlencoded::parse(query.as_bytes())
            .map(|(key, value)| (key.into_owned(), Value::String(value.into_owned())))
            .collect::<Members>()
            .into()
    }
}
