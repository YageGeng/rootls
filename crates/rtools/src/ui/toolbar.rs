//! The bottom toolbar of a plugin view and the parts it holds (spec §5.7).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, Styled as _, Window,
};

/// One row under a top border, laid out left to right; every plugin's
/// bottom bar has this height and spacing.
#[derive(Default, IntoElement)]
pub struct Toolbar {
    children: Vec<AnyElement>,
}

impl Toolbar {
    /// A quiet, icon-only button in the bar's style; name it with
    /// [`super::ButtonExt`].
    pub fn button(id: impl Into<ElementId>, icon: IconName) -> Button {
        Button::new(id).ghost().small().icon(icon)
    }

    /// The divider between groups of buttons.
    pub fn separator() -> Separator {
        Separator::vertical().h_4().mx_1()
    }
}

impl ParentElement for Toolbar {
    /// Appends elements to the row.
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Toolbar {
    /// Lays the children out in one bordered row.
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .children(self.children)
    }
}
