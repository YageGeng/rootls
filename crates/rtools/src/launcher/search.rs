//! Source recognition and filtering for the launcher's current input.

use super::{Launcher, Mode, Row, paste::PASTE_INLINE_LIMIT};
use crate::plugin::{ContentMatch, search_text};
use futures::lock::Mutex;
use gpui_kit::{AppContext as _, Context, SharedString, Task, Window};
use std::sync::Arc;

/// Recognition belongs to the source document; the editable filter can change independently.
#[derive(Default)]
pub(super) struct LauncherSearch {
    pub(super) query: SharedString,
    content: Vec<Option<ContentMatch>>,
    revision: u64,
    /// The active parser owns this permit until it returns, even when its caller is cancelled.
    recognition_slot: Arc<Mutex<()>>,
    pub(super) task: Option<Task<()>>,
    pub(super) confirm_pending: bool,
    pub(super) manual: Option<&'static str>,
}

impl Launcher {
    /// Refilters a chip immediately, or recognizes a new inline document.
    pub(super) fn query_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value();
        if query == self.search.query {
            return;
        }
        self.search.query = query;
        if self.pasted.is_some() {
            self.search.confirm_pending = false;
            self.search.manual = None;
            self.refresh_rows(cx);
            self.sync_window_size(window);
        } else {
            self.recognize(window, cx);
        }
    }

    /// Runs document checks once and fences background results against newer edits and pastes.
    pub(super) fn recognize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.revision += 1;
        self.search.task = None;
        self.search.confirm_pending = false;
        self.search.manual = None;
        let source = match &self.pasted {
            Some(paste) => Some(Arc::clone(&paste.text)),
            None if self.search.query.len() > PASTE_INLINE_LIMIT => {
                Some(Arc::new(self.search.query.to_string()))
            }
            None => None,
        };
        if let Some(source) = source {
            let revision = self.search.revision;
            let recognizers = self.registry.recognizers.clone();
            let slot = Arc::clone(&self.search.recognition_slot);
            self.search.content.clear();
            self.search.task = Some(cx.spawn(async move |this, cx| {
                // Replacing the foreground task drops its wait before it can dispatch obsolete work.
                let permit = slot.lock_owned().await;
                let content = cx
                    .background_spawn(async move {
                        let _permit = permit;
                        recognizers.detect(&source)
                    })
                    .await;
                // Keep the result even if the popup closed while the worker was reading the source.
                let Ok(Some(confirm)) = this.update(cx, |launcher, cx| {
                    if launcher.search.revision != revision {
                        return None;
                    }
                    launcher.search.task = None;
                    launcher.search.content = content;
                    launcher.refresh_rows(cx);
                    Some(std::mem::take(&mut launcher.search.confirm_pending))
                }) else {
                    return;
                };
                let _ = this.update_in(cx, |launcher, window, cx| {
                    if launcher.mode == Mode::List && launcher.search.revision == revision {
                        launcher.sync_window_size(window);
                        if confirm && launcher.search.query == launcher.query.read(cx).value() {
                            launcher.confirm(window, cx);
                        }
                    }
                });
            }));
        } else {
            self.search.content = self.registry.recognizers.detect(&self.search.query);
        }
        self.refresh_rows(cx);
        self.sync_window_size(window);
    }

    /// Derives rows and selection from the cached matches, retaining a user's selection by ID.
    pub(super) fn refresh_rows(&mut self, cx: &mut Context<Self>) {
        let needle = search_text(&self.search.query).to_lowercase();
        self.rows = self
            .registry
            .ranked(&needle, &self.search.content, self.pasted.is_some())
            .into_iter()
            .map(Row::Plugin)
            .collect();
        if self.pasted.is_none() {
            self.rows.extend(
                [Row::Settings, Row::Quit]
                    .into_iter()
                    .filter(|row| row.manifest(&self.registry).search_rank(&needle).is_some()),
            );
        }
        self.selected = self
            .search
            .manual
            .and_then(|id| {
                self.rows
                    .iter()
                    .position(|row| row.manifest(&self.registry).id == id)
            })
            .unwrap_or(0);
        cx.notify();
    }
}
