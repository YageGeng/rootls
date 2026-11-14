//! XML ↔ JSON conversion (spec §5.5).
//!
//! Mapping: attributes become `@name`, element text becomes `#text` (or a bare
//! string when the element holds only text), repeated children become arrays
//! and empty elements become `null`.

use std::collections::HashSet;
use std::fmt::{self, Write as _};

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::{escape, resolve_xml_entity};
use quick_xml::events::{BytesRef, BytesStart, Event};
use rust_i18n::t;
use serde_json::{Map, Value};

use super::Members;
use crate::plugins::json::core::{INDENT, JsonError};

/// Element name used when the JSON shape has no single natural root.
const ROOT: &str = "root";
/// Element name used for members of nested arrays.
const ITEM: &str = "item";
/// Deepest element nesting accepted, matching serde_json's own recursion limit.
const MAX_DEPTH: usize = 128;

/// XML source text to be read as JSON.
pub struct Xml<'a>(pub &'a str);

impl TryFrom<Xml<'_>> for Value {
    type Error = JsonError;

    /// Parses an XML document into JSON, keyed by the root element name.
    fn try_from(Xml(src): Xml<'_>) -> Result<Self, JsonError> {
        let mut reader = Reader::from_str(src);
        let mut tree = TreeBuilder::default();
        loop {
            match reader.read_event()? {
                Event::Start(start) => tree.open(Node::try_from(&start)?)?,
                Event::Empty(start) => tree.close(Node::try_from(&start)?)?,
                Event::End(_) => {
                    let node = tree.open.pop().ok_or_else(|| {
                        JsonError::Xml(t!("json.error.xml_extra_end").into_owned())
                    })?;
                    tree.close(node)?;
                }
                Event::Text(text) => tree.push_text(&text.xml10_content())?,
                Event::CData(data) => tree.push_text(&data.xml10_content())?,
                Event::GeneralRef(reference) => tree.push_text(&resolve_reference(&reference)?)?,
                Event::Eof => break,
                // Comments, processing instructions, declarations and doctypes carry no data.
                _ => {}
            }
        }
        tree.finish()
    }
}

/// Displays a JSON value as an indented XML document ending with a newline.
pub struct AsXml<'a>(pub &'a Value);

impl fmt::Display for AsXml<'_> {
    /// Writes the XML rendering of the wrapped value.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut writer = XmlWriter { out: f, depth: 0 };
        let value = self.0;
        match value {
            // A single non-array member names the document root itself.
            Value::Object(map) if map.len() == 1 && !map.values().all(Value::is_array) => {
                let (name, inner) = map.iter().next().expect("map has exactly one entry");
                writer.element(name, inner)
            }
            Value::Array(_) => {
                writer.line(format_args!("<{ROOT}>"))?;
                writer.depth += 1;
                writer.element(ITEM, value)?;
                writer.depth -= 1;
                writer.line(format_args!("</{ROOT}>"))
            }
            _ => writer.element(ROOT, value),
        }
    }
}

/// Resolves a character reference or one of the five predefined XML entities.
fn resolve_reference(reference: &BytesRef<'_>) -> Result<String, JsonError> {
    if reference.is_char_ref()
        && let Some(ch) = reference.resolve_char_ref()?
    {
        return Ok(ch.to_string());
    }
    resolve_xml_entity(reference)
        .map(str::to_string)
        .ok_or_else(|| {
            JsonError::Xml(t!("json.error.xml_unknown_entity", name = &**reference).into_owned())
        })
}

/// An element being read: its name, attributes, finished children and raw text.
struct Node {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<(String, Value)>,
    text: String,
}

impl TryFrom<&BytesStart<'_>> for Node {
    type Error = JsonError;

    /// Captures the name and attributes of a start or empty tag.
    fn try_from(start: &BytesStart<'_>) -> Result<Self, JsonError> {
        let name = start.name().as_ref().to_string();
        let mut attributes = Vec::new();
        for attribute in start.attributes() {
            let attribute = attribute?;
            let key = attribute.key.as_ref().to_string();
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)?
                .into_owned();
            attributes.push((key, value));
        }
        Ok(Self {
            name,
            attributes,
            children: Vec::new(),
            text: String::new(),
        })
    }
}

impl From<Node> for Value {
    /// Collapses the element into its JSON value per the spec mapping.
    fn from(node: Node) -> Self {
        let text = node.text.trim();
        if node.attributes.is_empty() && node.children.is_empty() {
            return if text.is_empty() {
                Value::Null
            } else {
                Value::String(text.to_string())
            };
        }
        let mut members: Members = node
            .attributes
            .into_iter()
            .map(|(key, value)| (format!("@{key}"), Value::String(value)))
            .collect();
        // Element values are never arrays, which `Members` relies on.
        members.extend(node.children);
        if !text.is_empty() {
            members.extend([("#text".to_string(), Value::String(text.to_string()))]);
        }
        members.into()
    }
}

/// Element stack and finished root collected while reading events.
#[derive(Default)]
struct TreeBuilder {
    open: Vec<Node>,
    root: Option<(String, Value)>,
}

impl TreeBuilder {
    /// Enters an element, refusing nesting deep enough to exhaust the stack later.
    fn open(&mut self, node: Node) -> Result<(), JsonError> {
        if self.open.len() >= MAX_DEPTH {
            return Err(JsonError::Xml(t!("json.error.xml_too_deep").into_owned()));
        }
        self.open.push(node);
        Ok(())
    }

    /// Attaches a finished element to its parent, or records it as the root.
    fn close(&mut self, mut node: Node) -> Result<(), JsonError> {
        // The name is not part of the element's value, so it can be moved out.
        let name = std::mem::take(&mut node.name);
        let value = Value::from(node);
        match self.open.last_mut() {
            Some(parent) => parent.children.push((name, value)),
            None if self.root.is_some() => {
                return Err(JsonError::Xml(
                    t!("json.error.xml_multiple_roots").into_owned(),
                ));
            }
            None => self.root = Some((name, value)),
        }
        Ok(())
    }

    /// Appends character data to the innermost open element.
    fn push_text(&mut self, text: &str) -> Result<(), JsonError> {
        match self.open.last_mut() {
            Some(node) => node.text.push_str(text),
            None if text.trim().is_empty() => {}
            None => {
                return Err(JsonError::Xml(
                    t!("json.error.xml_text_outside_root").into_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Returns `{root_name: value}` once every element has been closed.
    fn finish(self) -> Result<Value, JsonError> {
        if !self.open.is_empty() {
            return Err(JsonError::Xml(t!("json.error.xml_unclosed").into_owned()));
        }
        let (name, value) = self
            .root
            .ok_or_else(|| JsonError::Xml(t!("json.error.xml_no_root").into_owned()))?;
        let mut map = Map::new();
        map.insert(name, value);
        Ok(Value::Object(map))
    }
}

/// Writes indented XML straight into the output formatter.
struct XmlWriter<'a, 'f> {
    out: &'a mut fmt::Formatter<'f>,
    depth: usize,
}

impl XmlWriter<'_, '_> {
    /// Writes one line indented to the current depth.
    fn line(&mut self, content: fmt::Arguments<'_>) -> fmt::Result {
        writeln!(
            self.out,
            "{:width$}{content}",
            "",
            width = self.depth * INDENT
        )
    }

    /// Writes `value` as element(s) named `name`; arrays repeat the element.
    fn element(&mut self, name: &str, value: &Value) -> fmt::Result {
        let tag = xml_name(name);
        match value {
            Value::Null => self.line(format_args!("<{tag}/>")),
            Value::Array(items) => {
                for item in items {
                    if item.is_array() {
                        // A nested array becomes one element whose members are <item>s.
                        self.line(format_args!("<{tag}>"))?;
                        self.depth += 1;
                        self.element(ITEM, item)?;
                        self.depth -= 1;
                        self.line(format_args!("</{tag}>"))?;
                    } else {
                        self.element(name, item)?;
                    }
                }
                Ok(())
            }
            Value::Object(map) => self.object(&tag, map),
            scalar => self.line(format_args!(
                "<{tag}>{}</{tag}>",
                escape(scalar_text(scalar))
            )),
        }
    }

    /// Writes an object element: `@` keys as attributes, `#text` as text, the rest as children.
    fn object(&mut self, tag: &str, map: &Map<String, Value>) -> fmt::Result {
        let mut open = format!("<{tag}");
        let mut used = HashSet::new();
        for (key, value) in map {
            if let Some(attribute) = key.strip_prefix('@') {
                // Sanitizing can map distinct keys to one name; a duplicate attribute
                // would make the document malformed, so later ones get a suffix.
                let base = xml_name(attribute);
                let mut name = base.clone();
                let mut suffix = 2;
                while !used.insert(name.clone()) {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
                write!(open, " {name}=\"{}\"", escape(scalar_text(value)))?;
            }
        }
        let text = map.get("#text").map(scalar_text);
        let children: Vec<_> = map
            .iter()
            .filter(|(key, _)| !key.starts_with('@') && key.as_str() != "#text")
            .collect();
        match (children.is_empty(), text) {
            (true, None) => self.line(format_args!("{open}/>")),
            (true, Some(text)) => self.line(format_args!("{open}>{}</{tag}>", escape(&text))),
            (false, text) => {
                self.line(format_args!("{open}>"))?;
                self.depth += 1;
                if let Some(text) = text {
                    self.line(format_args!("{}", escape(&text)))?;
                }
                for (key, value) in children {
                    self.element(key, value)?;
                }
                self.depth -= 1;
                self.line(format_args!("</{tag}>"))
            }
        }
    }
}

/// Text form of a value used as element text or attribute value, with characters
/// that XML 1.0 forbids (C0 controls other than tab/LF/CR, U+FFFE, U+FFFF)
/// replaced by U+FFFD so the output stays well-formed.
fn scalar_text(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    text.chars()
        .map(|ch| match ch {
            '\t' | '\n' | '\r' => ch,
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => '\u{fffd}',
            _ => ch,
        })
        .collect()
}

/// Turns a JSON key into a well-formed XML name.
fn xml_name(key: &str) -> String {
    // Unicode letters, digits and `_ . - :` are legal XML name characters (`:` keeps
    // namespace prefixes); anything else becomes `_`.
    let mut name: String = key
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '_' | '.' | '-' | ':') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty()
        || name.starts_with(|ch: char| ch.is_ascii_digit() || matches!(ch, '-' | '.' | ':'))
    {
        name.insert(0, '_');
    }
    name
}
