//! Button tooltips that show in every rtools window.
//!
//! GPUI Kit's own button tooltips render through its window `Root`. The
//! Wayland launcher is a full-screen layer-shell overlay without one: a layer
//! surface keeps client-side decorations, so a `Root` would draw a window
//! frame and shadow across the whole screen. GPUI's built-in tooltip needs no
//! root, so buttons that can appear in the launcher take their tooltip from it.

use gpui_kit::component::button::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, Div, ElementId, InteractiveElement as _, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, div,
};

/// Tooltips that need no GPUI Kit `Root`; use them instead of
/// `Button::tooltip` on buttons the launcher shows. Apply them last: the
/// result is the element wrapping the button, not the button.
pub trait ButtonExt: Sized {
    /// Shows `text` while the pointer rests on the button.
    fn with_tooltip(self, text: impl Into<SharedString>) -> Stateful<Div> {
        self.with_key_tooltip(text, None)
    }

    /// Shows `text`, followed by the key bound to the action in the key
    /// context of `binding`, if given.
    fn with_key_tooltip(
        self,
        text: impl Into<SharedString>,
        binding: Option<(&dyn Action, &'static str)>,
    ) -> Stateful<Div>;
}

impl ButtonExt for Button {
    /// Wraps the button in an element carrying the tooltip; the text also
    /// names the icon-only button for assistive technology, which
    /// `Button::tooltip` would otherwise do.
    fn with_key_tooltip(
        self,
        text: impl Into<SharedString>,
        binding: Option<(&dyn Action, &'static str)>,
    ) -> Stateful<Div> {
        let text = text.into();
        let binding = binding.map(|(action, context)| (action.boxed_clone(), context));
        // Tooltip texts differ between the buttons of one bar, so the text
        // gives each wrapper a distinct id among its siblings.
        div()
            .id(ElementId::Name(text.clone()))
            .child(self.accessibility_label(text.clone()))
            .tooltip(move |window, cx| {
                Tooltip::new(text.clone())
                    .when_some(binding.as_ref(), |tooltip, (action, context)| {
                        tooltip.action(action.as_ref(), Some(context))
                    })
                    .build(window, cx)
            })
    }
}
