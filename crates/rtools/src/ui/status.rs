//! A status page standing in for content that cannot be shown.

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Hsla, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div,
};

/// A centered icon, title and optional muted detail, used where a result
/// would go: an error, an empty input, identical texts.
#[derive(IntoElement)]
pub struct StatusPage {
    icon: IconName,
    color: Hsla,
    title: SharedString,
    detail: Option<SharedString>,
}

impl StatusPage {
    /// A page with an `icon` in a theme `color` above its `title`.
    pub fn new(icon: IconName, color: Hsla, title: impl Into<SharedString>) -> Self {
        Self {
            icon,
            color,
            title: title.into(),
            detail: None,
        }
    }

    /// Adds a muted line under the title.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl RenderOnce for StatusPage {
    /// Fills the available space with the page centered in it.
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .px_6()
            .child(Icon::new(self.icon).size_5().text_color(self.color))
            .child(div().text_sm().child(self.title))
            .when_some(self.detail, |this, detail| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                )
            })
    }
}
