//! Plugin discovery, content recognition and stable launcher ordering.

use super::{Plugin, PluginManifest};
use gpui_kit::{App, SharedString};
use rust_i18n::t;

/// A content matcher can run without borrowing a plugin view, window or registry.
pub type ContentRecognizer = fn(&str) -> Option<ContentMatch>;

/// How confidently a plugin can handle the original input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentKind {
    Recognized,
    Hint,
    Fallback,
}

/// Language-independent evidence that can be cached across locale changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentMatch {
    pub kind: ContentKind,
    pub message_key: &'static str,
}

impl ContentMatch {
    /// Resolves the row's explanation using the current interface language.
    pub fn label(self) -> SharedString {
        t!(self.message_key).into()
    }
}

/// Whether to reuse the plugin view or deliver the original document when it opens.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchKind {
    Keyword,
    Content(ContentMatch),
}

/// One launcher result pointing back into the registry.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginMatch {
    pub index: usize,
    pub kind: MatchKind,
}

/// Lower ranks are more specific. Equal ranks retain registration order.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Rank {
    Trigger,
    Recognized,
    Exact,
    Prefix,
    Substring,
    Hint,
    Fallback,
}

impl From<ContentKind> for Rank {
    /// Interleaves structured recognition and generic suggestions with ordinary plugin searches.
    fn from(kind: ContentKind) -> Self {
        match kind {
            ContentKind::Recognized => Self::Recognized,
            ContentKind::Hint => Self::Hint,
            ContentKind::Fallback => Self::Fallback,
        }
    }
}

impl PluginManifest {
    /// Finds the strongest metadata match for a normalized search term.
    pub(crate) fn search_rank(&self, needle: &str) -> Option<Rank> {
        if needle.is_empty() {
            return Some(Rank::Exact);
        }
        if self
            .triggers
            .iter()
            .any(|word| word.trim().to_lowercase() == needle)
        {
            return Some(Rank::Trigger);
        }
        let title = t!(self.title);
        [self.id, title.as_ref()]
            .into_iter()
            .chain(self.triggers.iter().chain(self.keywords).copied())
            .filter_map(|label| {
                let label = label.trim().to_lowercase();
                if label == needle {
                    Some(Rank::Exact)
                } else if label.starts_with(needle) {
                    Some(Rank::Prefix)
                } else {
                    label.contains(needle).then_some(Rank::Substring)
                }
            })
            .min()
    }
}

/// Normalizes clipboard or keyboard input for matching while preserving the original for launch.
pub(crate) fn search_text(text: &str) -> &str {
    text.trim_start_matches('\u{feff}').trim()
}

/// A copyable list of pure callbacks aligned with the registry's immutable plugin indexes.
#[derive(Clone)]
pub(crate) struct Recognizers(Vec<Option<ContentRecognizer>>);

impl Recognizers {
    /// Runs each handler once per source, before any search terms are applied to its results.
    pub(crate) fn detect(&self, source: &str) -> Vec<Option<ContentMatch>> {
        if search_text(source).is_empty() {
            return Vec::new();
        }
        self.0
            .iter()
            .map(|f| f.and_then(|detect| detect(source)))
            .collect()
    }
}

/// Compiled plugins and their independent recognizers in registration order.
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    pub(crate) recognizers: Recognizers,
}

impl FromIterator<Box<dyn Plugin>> for PluginRegistry {
    /// Captures each recognizer once, so workers never have to hold a UI plugin.
    fn from_iter<I: IntoIterator<Item = Box<dyn Plugin>>>(iter: I) -> Self {
        let plugins: Vec<_> = iter.into_iter().collect();
        let recognizers = Recognizers(plugins.iter().map(|p| p.recognizer()).collect());
        Self {
            plugins,
            recognizers,
        }
    }
}

impl PluginRegistry {
    /// Registers every plugin's key bindings.
    pub fn bind_keys(&self, cx: &mut App) {
        for plugin in &self.plugins {
            plugin.bind_keys(cx);
        }
    }

    /// Resolves an index returned by this registry.
    pub fn plugin(&self, index: usize) -> &dyn Plugin {
        self.plugins[index].as_ref()
    }

    /// Searches a typed query using both metadata and the recognized content of the query itself.
    pub fn search(&self, query: &str) -> Vec<PluginMatch> {
        self.ranked(
            &search_text(query).to_lowercase(),
            &self.recognizers.detect(query),
            false,
        )
    }

    /// Lists every handler of a pasted document without interpreting its text as a command.
    pub fn search_content(&self, source: &str) -> Vec<PluginMatch> {
        self.ranked("", &self.recognizers.detect(source), true)
    }

    /// Ranks a normalized term once per plugin. A chip filter retains the original launch action.
    pub(crate) fn ranked(
        &self,
        needle: &str,
        content: &[Option<ContentMatch>],
        pasted: bool,
    ) -> Vec<PluginMatch> {
        let mut hits: Vec<_> = self
            .plugins
            .iter()
            .enumerate()
            .filter_map(|(index, plugin)| {
                let document = content
                    .get(index)
                    .and_then(Option::as_ref)
                    .map(|matched| (Rank::from(matched.kind), MatchKind::Content(*matched)));
                let (rank, kind) = if pasted {
                    let (content_rank, kind) = document?;
                    let rank = if needle.is_empty() {
                        content_rank
                    } else {
                        plugin.manifest().search_rank(needle)?
                    };
                    (rank, kind)
                } else {
                    let keyword = plugin
                        .manifest()
                        .search_rank(needle)
                        .map(|rank| (rank, MatchKind::Keyword));
                    keyword
                        .into_iter()
                        .chain(document)
                        .min_by_key(|(rank, _)| *rank)?
                };
                Some((rank, PluginMatch { index, kind }))
            })
            .collect();
        hits.sort_by_key(|(rank, _)| *rank);
        hits.into_iter().map(|(_, hit)| hit).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Filtering a pasted document limits the handlers while preserving the original launch action.
    #[test]
    fn pasted_filters_reuse_content_actions() {
        let registry: PluginRegistry = crate::plugins::builtins().into_iter().rev().collect();
        let content = registry.recognizers.detect("{\n  \"text\": \"fy\"\n}");
        for (filter, expected) in [
            ("", vec!["json", "translate"]),
            ("fy", vec!["translate"]),
            ("fanyi", vec!["translate"]),
            ("jqb", vec![]),
            ("", vec!["json", "translate"]),
        ] {
            let hits = registry.ranked(filter, &content, true);
            let ids: Vec<_> = hits
                .iter()
                .map(|hit| registry.plugin(hit.index).manifest().id)
                .collect();
            assert_eq!(ids, expected, "filter {filter:?}");
            assert!(
                hits.iter()
                    .all(|hit| matches!(hit.kind, MatchKind::Content(_)))
            );
        }
    }
}
