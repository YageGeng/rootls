//! YAML → JSON conversion that keeps number literals exactly as written.
//!
//! serde-saphyr hands numbers over as `i64`/`u64`/`f64`, which loses large
//! integers and spellings such as `1.50`. Every node is therefore read as
//! [`Spanned`], and number literals are re-read from the source text.

use std::fmt;
use std::str::FromStr;

use rust_i18n::t;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use serde_saphyr::Spanned;

use crate::plugins::json::core::JsonError;

/// YAML source text to be read as JSON.
pub struct Yaml<'a>(pub &'a str);

impl TryFrom<Yaml<'_>> for Value {
    type Error = JsonError;

    /// Converts a YAML document whose top level is a mapping or sequence.
    fn try_from(Yaml(src): Yaml<'_>) -> Result<Self, JsonError> {
        // Pasted documents may be far larger than the config files the default
        // budget is sized for; keep the depth and alias guards, lift the size caps.
        let options = serde_saphyr::options! {
            budget: serde_saphyr::budget! {
                max_nodes: usize::MAX,
                max_events: usize::MAX,
            },
        };
        let root: Spanned<Node> = serde_saphyr::from_str_with_options(src, options)
            .map_err(|err| JsonError::Yaml(err.to_string()))?;
        match root.value {
            Node::Seq(_) | Node::Map(_) => Ok(Source(src).value(root)),
            _ => Err(JsonError::Yaml(
                t!("json.error.yaml_top_level").into_owned(),
            )),
        }
    }
}

/// A YAML node as deserialized, before number literals are recovered.
enum Node {
    Null,
    Bool(bool),
    /// The parsed value, used when the literal is not valid JSON number syntax.
    Number(Number),
    String(String),
    Seq(Vec<Spanned<Node>>),
    Map(Vec<(Key, Spanned<Node>)>),
}

impl<'de> Deserialize<'de> for Node {
    /// Reads any YAML node.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

/// Builds [`Node`]s from serde callbacks.
struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    /// Describes the accepted input for serde error messages.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a YAML node")
    }

    /// Accepts booleans.
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Node, E> {
        Ok(Node::Bool(value))
    }

    /// Accepts negative integers.
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Node, E> {
        Ok(Node::Number(value.into()))
    }

    /// Accepts non-negative integers.
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Node, E> {
        Ok(Node::Number(value.into()))
    }

    /// Accepts floats; non-finite values have no JSON form and become `null`.
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Node, E> {
        Ok(Number::from_f64(value).map_or(Node::Null, Node::Number))
    }

    /// Accepts strings.
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Node, E> {
        Ok(Node::String(value.to_string()))
    }

    /// Accepts `null` / `~` / empty values.
    fn visit_unit<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    /// Accepts an absent optional value.
    fn visit_none<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::Null)
    }

    /// Accepts a present optional value.
    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        Node::deserialize(deserializer)
    }

    /// Accepts sequences.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Spanned<Node>>()? {
            items.push(item);
        }
        Ok(Node::Seq(items))
    }

    /// Accepts mappings, keeping key order.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let mut entries = Vec::new();
        while let Some(key) = map.next_key::<Key>()? {
            entries.push((key, map.next_value::<Spanned<Node>>()?));
        }
        Ok(Node::Map(entries))
    }
}

/// A mapping key; YAML allows scalar keys of any type, JSON needs their text.
struct Key(String);

impl<'de> Deserialize<'de> for Key {
    /// Reads a scalar key as text.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(KeyVisitor)
    }
}

/// Builds [`Key`]s from scalar callbacks.
struct KeyVisitor;

impl Visitor<'_> for KeyVisitor {
    type Value = Key;

    /// Describes the accepted input for serde error messages.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a scalar mapping key")
    }

    /// Accepts string keys.
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    /// Accepts boolean keys.
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    /// Accepts negative integer keys.
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    /// Accepts non-negative integer keys.
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    /// Accepts float keys.
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    /// Accepts a `null` key.
    fn visit_unit<E: de::Error>(self) -> Result<Key, E> {
        Ok(Key("null".into()))
    }
}

/// The YAML source, used to recover number literals by their byte span.
struct Source<'a>(&'a str);

impl Source<'_> {
    /// Converts a node, preferring the literal text of numbers when it is valid JSON.
    fn value(&self, node: Spanned<Node>) -> Value {
        match node.value {
            Node::Null => Value::Null,
            Node::Bool(value) => Value::Bool(value),
            Node::Number(parsed) => Value::Number(self.literal(&node.defined).unwrap_or(parsed)),
            Node::String(text) => Value::String(text),
            Node::Seq(items) => {
                Value::Array(items.into_iter().map(|item| self.value(item)).collect())
            }
            Node::Map(entries) => Value::Object(
                entries
                    .into_iter()
                    .map(|(Key(key), value)| (key, self.value(value)))
                    .collect::<Map<_, _>>(),
            ),
        }
    }

    /// Reads the scalar at `location` as a JSON number literal (`0x1F` and `1_000` are not).
    fn literal(&self, location: &serde_saphyr::Location) -> Option<Number> {
        let span = location.span();
        let start = usize::try_from(span.byte_offset()?).ok()?;
        let len = usize::try_from(span.byte_len()?).ok()?;
        let text = self.0.get(start..start.checked_add(len)?)?.trim();
        Number::from_str(text).ok()
    }
}
