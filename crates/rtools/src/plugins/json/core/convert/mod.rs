//! Conversions between JSON and other text formats.

pub mod query;
pub mod xml;
pub mod yaml;

use serde_json::{Map, Value};

/// JSON object members where a repeated key gathers its values into an array
/// kept at the key's first-seen position.
///
/// Values added must never be arrays themselves, otherwise an existing array
/// would be mistaken for an earlier repetition.
#[derive(Default)]
pub(crate) struct Members(Map<String, Value>);

impl Extend<(String, Value)> for Members {
    /// Adds members, merging repeated keys into arrays.
    fn extend<I: IntoIterator<Item = (String, Value)>>(&mut self, iter: I) {
        for (key, value) in iter {
            match self.0.get_mut(&key) {
                Some(Value::Array(items)) => items.push(value),
                Some(existing) => {
                    let first = existing.take();
                    *existing = Value::Array(vec![first, value]);
                }
                None => {
                    self.0.insert(key, value);
                }
            }
        }
    }
}

impl FromIterator<(String, Value)> for Members {
    /// Collects members, merging repeated keys into arrays.
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        let mut members = Self::default();
        members.extend(iter);
        members
    }
}

impl From<Members> for Value {
    /// Wraps the members into a JSON object.
    fn from(members: Members) -> Self {
        Value::Object(members.0)
    }
}
