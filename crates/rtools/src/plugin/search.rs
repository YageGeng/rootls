//! Closing GPUI Kit's built-in editor search panels on Esc.

use gpui_kit::component::input::{EditorState, Escape};
use gpui_kit::{Context, Entity, Focusable as _, Window};

/// A plugin view whose editors open GPUI Kit's built-in search panel.
/// Registering `cx.listener(Self::close_search)` with `capture_action` on the
/// view's root gives it the shared Esc behavior.
pub trait SearchPanels: Sized + 'static {
    /// The editors with a search panel; focus falls back to the first.
    fn search_editors(&self) -> impl IntoIterator<Item = &Entity<EditorState>>;

    /// Esc closes every open search panel and stops there, returning focus
    /// to the editor being typed in (else the first one with a panel), since
    /// focus inside a closed panel would be lost. Without an open panel Esc
    /// goes on, so the launcher leaves the plugin.
    ///
    /// GPUI Kit's inputs would otherwise take Esc to move focus and leave a
    /// panel open. The panel sits beside the editor's text element, so focus
    /// in it does not count as inside the editor, hence no per-editor test.
    fn close_search(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        let open: Vec<&Entity<EditorState>> = self
            .search_editors()
            .into_iter()
            .filter(|editor| editor.read(cx).search_session().open)
            .collect();
        let Some(&target) = open
            .iter()
            .find(|editor| editor.read(cx).focus_handle(cx).is_focused(window))
            .or(open.first())
        else {
            return;
        };
        for editor in &open {
            editor.update(cx, |state, cx| state.close_search(cx));
        }
        target.update(cx, |state, cx| state.focus(window, cx));
        // Capture listeners do not stop an action on their own: the editor
        // would pass Esc on and the launcher would leave the plugin too.
        cx.stop_propagation();
    }
}
