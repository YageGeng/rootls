//! One option of a dropdown: its label and the value it stands for.

use gpui_kit::SharedString;
use gpui_kit::component::searchable_list::SearchableListItem;

/// One option of a `Select`: the label shown in the menu and the trigger,
/// and the value choosing it selects.
#[derive(Clone)]
pub struct Choice<V> {
    pub label: SharedString,
    pub value: V,
}

impl<V: Clone + PartialEq + 'static> SearchableListItem for Choice<V> {
    type Value = V;

    /// The label shown in the menu and the trigger.
    fn title(&self) -> SharedString {
        self.label.clone()
    }

    /// The value this option selects.
    fn value(&self) -> &V {
        &self.value
    }
}
