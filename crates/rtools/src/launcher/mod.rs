//! The launcher popup: search, plugin list, paste chip and the mounted plugin
//! (spec §4.2, §5.7).

pub mod paste;

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex, window_paddings};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_i18n::t;

use crate::cli::ShellCommand;
use crate::plugin::{Launch, MatchKind, PluginManifest, PluginMatch, PluginRegistry, PluginView};
use crate::shell::Shell;
use paste::PastedInput;

actions!(launcher, [SelectPrev, SelectNext, Back, Hide, Detach]);

/// Key context of the list mode.
const LIST_CONTEXT: &str = "LauncherList";
/// Key context of the plugin mode.
const PLUGIN_CONTEXT: &str = "LauncherPlugin";

// Window geometry in rem (window sizes are a platform boundary; spec §5.7).
const WIDTH: f32 = 50.;
const SEARCH_ROW: f32 = 3.25;
const GROUP_LABEL: f32 = 2.;
const ROW: f32 = 3.5;
const LIST_PADDING: f32 = 0.5;
const FOOTER: f32 = 2.75;
const PASTE_ROW: f32 = 2.25;
const EMPTY_STATE: f32 = 6.;
const PLUGIN_HEIGHT: f32 = 37.5;
/// Smallest size the plugin panel can be resized to, in rem.
const MIN_WIDTH: f32 = 30.;
const MIN_HEIGHT: f32 = 20.;
/// Thickness of the resize zones just outside the panel border, in rem.
const RESIZE_ZONE: f32 = 0.5;

/// Detach shortcut; this GPUI version has no `secondary` modifier alias.
#[cfg(target_os = "macos")]
const DETACH_KEY: &str = "cmd-d";
/// Detach shortcut; this GPUI version has no `secondary` modifier alias.
#[cfg(not(target_os = "macos"))]
const DETACH_KEY: &str = "ctrl-d";

/// The settings row, listed and matched like a plugin.
static SETTINGS: PluginManifest = PluginManifest {
    id: "settings",
    title: "app.settings_title",
    description: "app.settings_detail",
    icon: IconName::Settings,
    keywords: &["settings", "preferences", "rtools"],
};

/// The quit row, listed and matched like a plugin.
static QUIT: PluginManifest = PluginManifest {
    id: "quit",
    title: "app.quit_title",
    description: "app.quit_detail",
    icon: IconName::Power,
    keywords: &["quit", "exit", "rtools"],
};

/// Installs the launcher key bindings.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectPrev, Some(LIST_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(LIST_CONTEXT)),
        KeyBinding::new("escape", Hide, Some(LIST_CONTEXT)),
        KeyBinding::new("escape", Back, Some(PLUGIN_CONTEXT)),
        KeyBinding::new(DETACH_KEY, Detach, Some(PLUGIN_CONTEXT)),
    ]);
}

/// Payload of a panel drag inside the full-screen overlay.
struct PanelDrag;

/// Payload of a panel resize inside the full-screen overlay: the edge held.
struct PanelResize(ResizeEdge);

/// One selectable launcher row.
enum Row {
    Plugin(PluginMatch),
    /// Opens the settings window.
    Settings,
    /// Exits the background process.
    Quit,
}

impl Row {
    /// The manifest the row is listed, matched and drawn from.
    fn manifest<'a>(&self, registry: &'a PluginRegistry) -> &'a PluginManifest {
        match self {
            Row::Plugin(hit) => registry.plugin(hit.index).manifest(),
            Row::Settings => &SETTINGS,
            Row::Quit => &QUIT,
        }
    }
}

/// What the popup currently shows.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    List,
    /// Registry index of the mounted plugin.
    Plugin(usize),
}

/// The launcher view; one entity lives for the whole process.
pub(crate) struct Launcher {
    registry: Rc<PluginRegistry>,
    data_root: PathBuf,
    query: Entity<InputState>,
    pasted: Option<PastedInput>,
    rows: Vec<Row>,
    selected: usize,
    mode: Mode,
    /// Plugin views by registry index, kept while the popup is hidden.
    views: HashMap<usize, PluginView>,
    _query_events: Subscription,
    /// Blur watcher of the current popup window.
    activation: Option<Subscription>,
    /// Drawn as a movable panel inside a full-screen overlay (Wayland, where a
    /// layer-shell surface cannot be moved) rather than filling its window.
    overlay: bool,
    /// Where a drag left the panel's top-left corner; `None` keeps it centered.
    origin: Option<Point<Pixels>>,
    /// Height in rem the list is centered on: its height when it was last
    /// shown, so the search row stays put while typing changes the rows.
    list_rems: f32,
    /// The size the plugin panel was resized to; `None` keeps the default.
    /// Kept while rtools runs; the list always has its own default width.
    size: Option<Size<Pixels>>,
    /// Pointer position at the previous drag step.
    drag_from: Point<Pixels>,
}

impl Launcher {
    /// Size used before the first layout; `attach` resizes to the content.
    pub const INITIAL_SIZE: Size<Pixels> = Size {
        width: px(800.),
        height: px(320.),
    };

    /// Creates the launcher inside its first popup window.
    pub fn new(
        registry: Rc<PluginRegistry>,
        data_root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // `attach` sets the placeholder, since it runs right after creation.
        let query = cx.new(|cx| InputState::new(window, cx));
        let query_events =
            cx.subscribe_in(&query, window, |this, _, event, window, cx| match event {
                // Content rows depend only on the paste, computed once in `intercept_paste`.
                InputEvent::Change if this.pasted.is_none() => {
                    this.refresh_rows(cx);
                    this.sync_window_size(window);
                }
                InputEvent::PressEnter { .. } => this.confirm(window, cx),
                _ => {}
            });
        let mut launcher = Self {
            registry,
            data_root,
            query,
            pasted: None,
            rows: Vec::new(),
            selected: 0,
            mode: Mode::List,
            views: HashMap::new(),
            _query_events: query_events,
            activation: None,
            overlay: false,
            origin: None,
            list_rems: 0.,
            size: None,
            drag_from: Point::default(),
        };
        launcher.refresh_rows(cx);
        launcher
    }

    /// Binds the launcher to a freshly opened window: a full-screen `overlay` it
    /// draws its panel into, or a popup it fills. Every opening starts centered.
    pub fn attach(&mut self, overlay: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = overlay;
        self.origin = None;
        self.list_rems = self.content_rems();
        // The language may have changed while the popup was hidden.
        self.sync_placeholder(window, cx);
        self.activation = Some(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.close(window, cx);
            }
        }));
        match self.mode {
            Mode::List => self.focus_query(window, cx),
            Mode::Plugin(index) => self.focus_plugin(index, window, cx),
        }
        self.sync_window_size(window);
    }

    /// Shows the search box hint for the paste state, in the current language.
    fn sync_placeholder(&self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = if self.pasted.is_some() {
            t!("launcher.pasted_placeholder")
        } else {
            t!("launcher.placeholder")
        };
        self.query.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx);
        });
    }

    /// Focuses the search box and selects its text.
    fn focus_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
    }

    /// Focuses the mounted plugin's editor.
    fn focus_plugin(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.views.get(&index) {
            window.focus(&view.focus, cx);
        }
    }

    /// Recomputes the rows for the query or the pasted text.
    fn refresh_rows(&mut self, cx: &mut Context<Self>) {
        self.rows = match &self.pasted {
            Some(pasted) => self
                .registry
                .search_content(pasted.text())
                .into_iter()
                .map(Row::Plugin)
                .collect(),
            None => {
                let query = self.query.read(cx).value();
                let needle = query.trim().to_lowercase();
                let mut rows: Vec<Row> = self
                    .registry
                    .search(&query)
                    .into_iter()
                    .map(Row::Plugin)
                    .collect();
                rows.extend(
                    [Row::Settings, Row::Quit]
                        .into_iter()
                        .filter(|row| row.manifest(&self.registry).matches_keyword(&needle)),
                );
                rows
            }
        };
        self.selected = 0;
        cx.notify();
    }

    /// Content height in rem for the current mode and rows.
    fn content_rems(&self) -> f32 {
        if let Mode::Plugin(_) = self.mode {
            return PLUGIN_HEIGHT;
        }
        let plugin_rows = self
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Plugin(_)))
            .count();
        let has_system = plugin_rows < self.rows.len();
        let groups = usize::from(plugin_rows > 0) + usize::from(has_system);
        let list = if self.rows.is_empty() {
            EMPTY_STATE
        } else {
            groups as f32 * GROUP_LABEL + self.rows.len() as f32 * ROW + LIST_PADDING
        };
        let paste = if self.pasted.is_some() { PASTE_ROW } else { 0. };
        paste + SEARCH_ROW + list + FOOTER
    }

    /// Resizes the popup window to fit the content plus the client-side shadow
    /// inset; the overlay instead sizes its panel while rendering.
    fn sync_window_size(&self, window: &mut Window) {
        if self.overlay {
            return;
        }
        let rem = window.rem_size();
        let paddings = window_paddings(window);
        let target = size(
            rem * WIDTH + paddings.left + paddings.right,
            rem * self.content_rems() + paddings.top + paddings.bottom,
        );
        if window.viewport_size() != target {
            window.resize(target);
        }
    }

    /// The panel inside the overlay, centered on the screen: the plugin panel
    /// on its height, the list on its height when it was shown (`list_rems`);
    /// drags move and resize it, and it stays on screen.
    fn panel_bounds(&self, window: &Window) -> Bounds<Pixels> {
        let rem = window.rem_size();
        let viewport = window.viewport_size();
        let plugin = self
            .size
            .unwrap_or_else(|| size(rem * WIDTH, rem * PLUGIN_HEIGHT));
        // Only the plugin panel is resizable; the list keeps its own width.
        let extent = match self.mode {
            Mode::Plugin(_) => plugin,
            Mode::List => size(rem * WIDTH, rem * self.content_rems()),
        };
        let size = size(
            extent.width.min(viewport.width),
            extent.height.min(viewport.height),
        );
        let centered_on = match self.mode {
            Mode::Plugin(_) => size.height,
            Mode::List => rem * self.list_rems,
        };
        let origin = self.origin.unwrap_or_else(|| {
            point(
                (viewport.width - size.width) / 2.,
                (viewport.height - centered_on) / 2.,
            )
        });
        let max_x = (viewport.width - size.width).max(px(0.));
        let max_y = (viewport.height - size.height).max(px(0.));
        Bounds::new(
            point(origin.x.clamp(px(0.), max_x), origin.y.clamp(px(0.), max_y)),
            size,
        )
    }

    /// Moves the dragged panel by the pointer's travel since the previous step.
    fn drag_panel(&mut self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        self.origin = Some(self.panel_bounds(window).origin + (position - self.drag_from));
        // Stored clamped, so dragging back from a screen edge responds at once.
        self.origin = Some(self.panel_bounds(window).origin);
        self.drag_from = position;
        cx.notify();
    }

    /// Moves the panel edges a resize drag holds by the pointer's travel since
    /// the previous step, keeping the minimum size and the screen bounds.
    fn resize_panel(
        &mut self,
        edge: ResizeEdge,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let rem = window.rem_size();
        let viewport = window.viewport_size();
        let min = size(rem * MIN_WIDTH, rem * MIN_HEIGHT);
        let delta = position - self.drag_from;
        self.drag_from = position;
        let Bounds {
            mut origin,
            size: mut extent,
        } = self.panel_bounds(window);
        if matches!(
            edge,
            ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
        ) {
            let dx = delta.x.max(-origin.x).min(extent.width - min.width);
            origin.x += dx;
            extent.width -= dx;
        }
        if matches!(
            edge,
            ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
        ) {
            extent.width = (extent.width + delta.x)
                .min(viewport.width - origin.x)
                .max(min.width);
        }
        if matches!(
            edge,
            ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
        ) {
            let dy = delta.y.max(-origin.y).min(extent.height - min.height);
            origin.y += dy;
            extent.height -= dy;
        }
        if matches!(
            edge,
            ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
        ) {
            extent.height = (extent.height + delta.y)
                .min(viewport.height - origin.y)
                .max(min.height);
        }
        self.origin = Some(origin);
        self.size = Some(extent);
        cx.notify();
    }

    /// The grab zones just outside the plugin panel's border that resize it;
    /// corners come last so they win where they overlap the edges.
    fn resize_zones(
        &self,
        panel: Bounds<Pixels>,
        rem: Pixels,
        cx: &Context<Self>,
    ) -> [Stateful<Div>; 8] {
        let band = rem * RESIZE_ZONE;
        let (left, top) = (panel.left() - band, panel.top() - band);
        let (right, bottom) = (panel.right(), panel.bottom());
        let (width, height) = (panel.size.width, panel.size.height);
        let corner = size(band * 2., band * 2.);
        [
            (
                ResizeEdge::Top,
                point(panel.left(), top),
                size(width, band),
                CursorStyle::ResizeUpDown,
            ),
            (
                ResizeEdge::Bottom,
                point(panel.left(), bottom),
                size(width, band),
                CursorStyle::ResizeUpDown,
            ),
            (
                ResizeEdge::Left,
                point(left, panel.top()),
                size(band, height),
                CursorStyle::ResizeLeftRight,
            ),
            (
                ResizeEdge::Right,
                point(right, panel.top()),
                size(band, height),
                CursorStyle::ResizeLeftRight,
            ),
            (
                ResizeEdge::TopLeft,
                point(left, top),
                corner,
                CursorStyle::ResizeUpLeftDownRight,
            ),
            (
                ResizeEdge::TopRight,
                point(right - band, top),
                corner,
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                ResizeEdge::BottomLeft,
                point(left, bottom - band),
                corner,
                CursorStyle::ResizeUpRightDownLeft,
            ),
            (
                ResizeEdge::BottomRight,
                point(right - band, bottom - band),
                corner,
                CursorStyle::ResizeUpLeftDownRight,
            ),
        ]
        .map(|(edge, origin, extent, cursor)| {
            div()
                .id(SharedString::from(format!("resize-{edge:?}")))
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(extent.width)
                .h(extent.height)
                .cursor(cursor)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, _| {
                        this.drag_from = event.position;
                    }),
                )
                .on_drag(PanelResize(edge), |_, _, _, cx| cx.new(|_| EmptyView))
        })
    }

    /// A stretch of the header or footer that moves the launcher: the panel
    /// inside the overlay, or the popup window itself elsewhere.
    fn drag_handle(&self, id: &'static str, cx: &Context<Self>) -> Stateful<Div> {
        let handle = h_flex().id(id);
        if self.overlay {
            handle
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, _| {
                        this.drag_from = event.position;
                    }),
                )
                .on_drag(PanelDrag, |_, _, _, cx| cx.new(|_| EmptyView))
        } else {
            handle.on_mouse_down(MouseButton::Left, |_, window, _| {
                window.start_window_move();
            })
        }
    }

    /// Keeps a multi-line or large paste out of the single-line search box.
    fn intercept_paste(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pasted) = PastedInput::intercept(text) else {
            return false;
        };
        self.pasted = Some(pasted);
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.sync_placeholder(window, cx);
        self.refresh_rows(cx);
        self.sync_window_size(window);
        true
    }

    /// Drops the paste chip and returns to keyword search.
    fn clear_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pasted = None;
        self.sync_placeholder(window, cx);
        self.refresh_rows(cx);
        self.sync_window_size(window);
    }

    /// Moves the selection up, wrapping around.
    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        if !self.rows.is_empty() {
            self.selected = (self.selected + self.rows.len() - 1) % self.rows.len();
            cx.notify();
        }
    }

    /// Moves the selection down, wrapping around.
    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.rows.is_empty() {
            self.selected = (self.selected + 1) % self.rows.len();
            cx.notify();
        }
    }

    /// Activates the selected row (Enter in the search box, or a click).
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.get(self.selected) {
            Some(Row::Settings) => {
                self.close(window, cx);
                cx.defer(Shell::open_settings);
            }
            Some(Row::Quit) => Shell::run(ShellCommand::Quit, cx),
            Some(Row::Plugin(hit)) => {
                let hit = hit.clone();
                self.open_plugin(&hit, window, cx);
            }
            None => {}
        }
    }

    /// Mounts a plugin: keyword hits reuse the cached view, content hits get a
    /// fresh view seeded with the text (pasted text or the query).
    fn open_plugin(&mut self, hit: &PluginMatch, window: &mut Window, cx: &mut Context<Self>) {
        let plugin = self.registry.plugin(hit.index);
        let id = plugin.manifest().id;
        let input = match hit.kind {
            MatchKind::Keyword => None,
            MatchKind::Content(_) => Some(match self.pasted.take() {
                Some(pasted) => pasted.into_text(),
                None => self.query.read(cx).value().to_string(),
            }),
        };
        if input.is_some() || !self.views.contains_key(&hit.index) {
            let data_dir = self.data_root.join(id);
            if let Err(err) = std::fs::create_dir_all(&data_dir) {
                tracing::warn!(
                    "creating the plugin data directory {} failed: {}",
                    data_dir.display(),
                    err
                );
            }
            let view = plugin.open(Launch { input, data_dir }, window, cx);
            self.views.insert(hit.index, view);
        }
        tracing::info!("opened plugin {}", id);
        self.pasted = None;
        self.mode = Mode::Plugin(hit.index);
        self.focus_plugin(hit.index, window, cx);
        self.sync_window_size(window);
        cx.notify();
    }

    /// Returns from a plugin to the list.
    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::List;
        self.clear_paste(window, cx);
        self.list_rems = self.content_rems();
        self.focus_query(window, cx);
        cx.notify();
    }

    /// Esc on the list: clear the paste chip first, otherwise hide.
    fn hide(&mut self, _: &Hide, window: &mut Window, cx: &mut Context<Self>) {
        if self.pasted.is_some() {
            self.clear_paste(window, cx);
        } else {
            self.close(window, cx);
        }
    }

    /// Moves the mounted plugin into its own window and hides the popup.
    fn detach(&mut self, _: &Detach, window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Plugin(index) = self.mode else {
            return;
        };
        let title: SharedString = t!(self.registry.plugin(index).manifest().title).into();
        let Some(view) = self.views.remove(&index) else {
            return;
        };
        self.mode = Mode::List;
        self.close(window, cx);
        cx.defer(move |cx| Shell::detach(view, title, cx));
    }

    /// Closes the popup window; the launcher entity and its state survive.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activation = None;
        window.remove_window();
        Shell::forget_popup(cx);
    }

    /// Search row, list or empty state, and the key-hint footer.
    fn render_list(&self, cx: &Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let theme = cx.theme();
        let search = h_flex()
            .h(rems(SEARCH_ROW))
            .px_3()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Input::new(&self.query)
                    .appearance(false)
                    .prefix(Icon::new(IconName::Search).text_color(theme.muted_foreground))
                    .on_paste(move |item, window, cx| {
                        let Some(text) = item.text() else {
                            return false;
                        };
                        weak.update(cx, |this, cx| this.intercept_paste(text, window, cx))
                            .unwrap_or(false)
                    }),
            );
        let hint = |key: &'static str, label: &'static str| {
            h_flex()
                .gap_1()
                .child(Kbd::new(Keystroke::parse(key).expect("valid keystroke")))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!(label).to_string()),
                )
        };
        let footer = self
            .drag_handle("list-drag", cx)
            .h(rems(FOOTER))
            .px_3()
            .gap_4()
            .border_t_1()
            .border_color(theme.border)
            .child(hint("enter", "launcher.open"))
            .child(hint(
                "escape",
                if self.pasted.is_some() {
                    "launcher.clear_paste"
                } else {
                    "launcher.hide"
                },
            ));
        v_flex()
            .size_full()
            .key_context(LIST_CONTEXT)
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::hide))
            .when_some(self.pasted.as_ref(), |this, pasted| {
                this.child(self.render_paste_chip(pasted, cx))
            })
            .child(search)
            .child(self.render_rows(cx))
            .child(footer)
            .into_any_element()
    }

    /// The "Pasted N KB" chip with a one-line preview and a clear button.
    fn render_paste_chip(&self, pasted: &PastedInput, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .h(rems(PASTE_ROW))
            .px_3()
            .pt_2()
            .gap_2()
            .child(
                Tag::new()
                    .child(t!("launcher.pasted_size", size = pasted.size_label()).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(pasted.preview()),
            )
            .child(
                Button::new("clear-paste")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(t!("launcher.clear_paste_tooltip").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.clear_paste(window, cx))),
            )
    }

    /// Grouped rows, or the empty state.
    fn render_rows(&self, cx: &Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        if self.rows.is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child(t!("launcher.no_match").to_string())
                .into_any_element();
        }
        let group_label = |key: &'static str| {
            div()
                .h(rems(GROUP_LABEL))
                .px_3()
                .flex()
                .items_center()
                .text_xs()
                .text_color(muted)
                .child(t!(key).to_string())
        };
        let plugin_group = if self.pasted.is_some() {
            "launcher.group_content"
        } else {
            "launcher.group_plugins"
        };
        let mut list = v_flex().flex_1().min_h_0().py_1().px_1();
        let (mut plugins_started, mut system_started) = (false, false);
        for (index, row) in self.rows.iter().enumerate() {
            match row {
                Row::Plugin(_) if !plugins_started => {
                    plugins_started = true;
                    list = list.child(group_label(plugin_group));
                }
                Row::Settings | Row::Quit if !system_started => {
                    system_started = true;
                    list = list.child(group_label("launcher.group_system"));
                }
                _ => {}
            }
            list = list.child(self.render_row(index, row, cx));
        }
        list.into_any_element()
    }

    /// One row: bordered icon tile, title and secondary line.
    fn render_row(&self, index: usize, row: &Row, cx: &Context<Self>) -> impl IntoElement {
        let manifest = row.manifest(&self.registry);
        let (title, detail): (SharedString, SharedString) = match row {
            Row::Plugin(PluginMatch {
                kind: MatchKind::Content(reason),
                ..
            }) => (
                t!("launcher.open_with", name = t!(manifest.title)).into(),
                reason.clone(),
            ),
            _ => (t!(manifest.title).into(), t!(manifest.description).into()),
        };
        let theme = cx.theme();
        ListItem::new(manifest.id)
            .h(rems(ROW))
            .selected(index == self.selected)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.confirm(window, cx);
            }))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .size_8()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_1()
                            .border_color(theme.border)
                            .rounded(theme.radius)
                            .child(Icon::new(manifest.icon).size_4()),
                    )
                    .child(
                        v_flex().child(div().text_sm().child(title)).child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        ),
                    ),
            )
    }

    /// Plugin header (back, icon, title, detach) above the mounted view.
    fn render_plugin(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let manifest = self.registry.plugin(index).manifest();
        let theme = cx.theme();
        let detach_key = Kbd::format(&Keystroke::parse(DETACH_KEY).expect("valid keystroke"));
        let detach_hint = t!("launcher.detach", key = detach_key).to_string();
        let header = h_flex()
            .h_10()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Button::new("back")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowLeft)
                    .tooltip(t!("launcher.back").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.back(&Back, window, cx))),
            )
            .child(
                self.drag_handle("plugin-drag", cx)
                    .flex_1()
                    .h_full()
                    .gap_2()
                    .child(
                        Icon::new(manifest.icon)
                            .size_4()
                            .text_color(theme.muted_foreground),
                    )
                    .child(div().text_sm().child(t!(manifest.title).to_string())),
            )
            .child(
                Button::new("detach")
                    .ghost()
                    .small()
                    .icon(IconName::AppWindow)
                    .tooltip(detach_hint)
                    .on_click(cx.listener(|this, _, window, cx| this.detach(&Detach, window, cx))),
            );
        let body = self.views.get(&index).map(|view| view.view.clone());
        v_flex()
            .size_full()
            .key_context(PLUGIN_CONTEXT)
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::detach))
            .child(header)
            .child(div().flex_1().min_h_0().children(body))
            .into_any_element()
    }
}

impl Render for Launcher {
    /// Shows the list or the mounted plugin; in the overlay, as a framed panel
    /// that a click beside dismisses.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.mode {
            Mode::List => self.render_list(cx),
            Mode::Plugin(index) => self.render_plugin(index, cx),
        };
        if !self.overlay {
            return content;
        }
        // The overlay window has no GPUI Kit root, which would otherwise set
        // the rem size, the frame, the surface and the text style.
        window.set_rem_size(cx.theme().font_size);
        let theme = cx.theme();
        let rem = window.rem_size();
        let panel = self.panel_bounds(window);
        let resizable = matches!(self.mode, Mode::Plugin(_));
        div()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    // The resize zones around the border count as the panel.
                    let grab = if resizable { rem * RESIZE_ZONE } else { px(0.) };
                    if !this
                        .panel_bounds(window)
                        .dilate(grab)
                        .contains(&event.position)
                    {
                        this.close(window, cx);
                    }
                }),
            )
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<PanelDrag>, window, cx| {
                    this.drag_panel(event.event.position, window, cx);
                }),
            )
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<PanelResize>, window, cx| {
                    let edge = event.drag(cx).0;
                    this.resize_panel(edge, event.event.position, window, cx);
                }),
            )
            .child(
                v_flex()
                    .absolute()
                    .left(panel.origin.x)
                    .top(panel.origin.y)
                    .w(panel.size.width)
                    .h(panel.size.height)
                    .overflow_hidden()
                    .rounded(theme.window_radius)
                    .border_1()
                    .border_color(theme.window_border)
                    .bg(theme.background)
                    .shadow(theme.shadow_tokens().lg)
                    .font_family(theme.font_family.clone())
                    .text_color(theme.foreground)
                    .child(content),
            )
            .when(resizable, |this| {
                this.children(self.resize_zones(panel, rem, cx))
            })
            .into_any_element()
    }
}
