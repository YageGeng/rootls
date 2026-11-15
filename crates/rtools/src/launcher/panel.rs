//! Where the launcher panel sits inside the Wayland overlay (spec §4.2): it
//! is centered on every opening, moved by dragging its header or footer,
//! resized by dragging the plugin panel's border, and kept on screen.

use gpui_kit::{Bounds, CursorStyle, Pixels, Point, ResizeEdge, Size, Window, point, px, size};

/// Width of the list and default width of the plugin panel, in rem.
pub(super) const WIDTH: f32 = 50.;
/// Default height of the plugin panel, in rem.
pub(super) const PLUGIN_HEIGHT: f32 = 37.5;
/// Smallest size the plugin panel can be resized to, in rem; the width keeps
/// the text diff's compare toolbar on one row in both languages.
const MIN_WIDTH: f32 = 35.;
const MIN_HEIGHT: f32 = 20.;
/// Thickness of the resize zones just outside the panel border, in rem.
pub(super) const RESIZE_ZONE: f32 = 0.5;

/// What the panel shows, which decides its size and what it centers on.
#[derive(Clone, Copy)]
pub(super) enum Content {
    /// A mounted plugin: the resizable plugin panel.
    Plugin,
    /// The list, this many rem tall.
    List(f32),
}

/// A grab zone along the plugin panel's border that resizes it.
pub(super) struct ResizeZone {
    pub id: &'static str,
    pub edge: ResizeEdge,
    pub bounds: Bounds<Pixels>,
    pub cursor: CursorStyle,
}

/// The panel's placement: kept while the popup is hidden, except that every
/// opening centers it again.
#[derive(Default)]
pub(super) struct Panel {
    /// Where a drag left the top-left corner; `None` keeps it centered.
    origin: Option<Point<Pixels>>,
    /// The size the plugin panel was resized to; `None` keeps the default.
    /// Kept while rtools runs; the list always has its own default width.
    size: Option<Size<Pixels>>,
    /// Height in rem the list is centered on: its height when it was last
    /// shown, so the search row stays put while typing changes the rows.
    list_rems: f32,
    /// Pointer position at the previous drag step.
    drag_from: Point<Pixels>,
}

impl Panel {
    /// Centers the panel again.
    pub fn recenter(&mut self) {
        self.origin = None;
    }

    /// Centers the list on a height of `rems` from now on.
    pub fn anchor_list(&mut self, rems: f32) {
        self.list_rems = rems;
    }

    /// Starts a move or resize drag at the pointer `position`.
    pub fn grab(&mut self, position: Point<Pixels>) {
        self.drag_from = position;
    }

    /// The panel's bounds in the overlay for `content`: centered (the plugin
    /// panel on its own height, the list on its anchor height) unless moved,
    /// and always on screen.
    pub fn bounds(&self, content: Content, window: &Window) -> Bounds<Pixels> {
        let rem = window.rem_size();
        let viewport = window.viewport_size();
        // Only the plugin panel is resizable; the list keeps its own width.
        let extent = match content {
            Content::Plugin => self
                .size
                .unwrap_or_else(|| size(rem * WIDTH, rem * PLUGIN_HEIGHT)),
            Content::List(rems) => size(rem * WIDTH, rem * rems),
        };
        let size = size(
            extent.width.min(viewport.width),
            extent.height.min(viewport.height),
        );
        let centered_on = match content {
            Content::Plugin => size.height,
            Content::List(_) => rem * self.list_rems,
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

    /// Moves the panel by the pointer's travel since the previous step.
    pub fn drag(&mut self, position: Point<Pixels>, content: Content, window: &Window) {
        self.origin = Some(self.bounds(content, window).origin + (position - self.drag_from));
        // Stored clamped, so dragging back from a screen edge responds at once.
        self.origin = Some(self.bounds(content, window).origin);
        self.drag_from = position;
    }

    /// Moves the plugin panel edges `edge` holds by the pointer's travel since
    /// the previous step, keeping the minimum size and the screen bounds.
    pub fn resize(&mut self, edge: ResizeEdge, position: Point<Pixels>, window: &Window) {
        let rem = window.rem_size();
        let viewport = window.viewport_size();
        let min = size(rem * MIN_WIDTH, rem * MIN_HEIGHT);
        let delta = position - self.drag_from;
        self.drag_from = position;
        let Bounds {
            mut origin,
            size: mut extent,
        } = self.bounds(Content::Plugin, window);
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
    }

    /// The grab zones just outside the plugin panel's border; corners come
    /// last so they win where they overlap the edges.
    pub fn resize_zones(&self, window: &Window) -> [ResizeZone; 8] {
        let panel = self.bounds(Content::Plugin, window);
        let band = window.rem_size() * RESIZE_ZONE;
        let (left, top) = (panel.left() - band, panel.top() - band);
        let (right, bottom) = (panel.right(), panel.bottom());
        let (width, height) = (panel.size.width, panel.size.height);
        let corner = size(band * 2., band * 2.);
        let zone = |id, edge, origin, extent, cursor| ResizeZone {
            id,
            edge,
            bounds: Bounds::new(origin, extent),
            cursor,
        };
        [
            zone(
                "resize-top",
                ResizeEdge::Top,
                point(panel.left(), top),
                size(width, band),
                CursorStyle::ResizeUpDown,
            ),
            zone(
                "resize-bottom",
                ResizeEdge::Bottom,
                point(panel.left(), bottom),
                size(width, band),
                CursorStyle::ResizeUpDown,
            ),
            zone(
                "resize-left",
                ResizeEdge::Left,
                point(left, panel.top()),
                size(band, height),
                CursorStyle::ResizeLeftRight,
            ),
            zone(
                "resize-right",
                ResizeEdge::Right,
                point(right, panel.top()),
                size(band, height),
                CursorStyle::ResizeLeftRight,
            ),
            zone(
                "resize-top-left",
                ResizeEdge::TopLeft,
                point(left, top),
                corner,
                CursorStyle::ResizeUpLeftDownRight,
            ),
            zone(
                "resize-top-right",
                ResizeEdge::TopRight,
                point(right - band, top),
                corner,
                CursorStyle::ResizeUpRightDownLeft,
            ),
            zone(
                "resize-bottom-left",
                ResizeEdge::BottomLeft,
                point(left, bottom - band),
                corner,
                CursorStyle::ResizeUpRightDownLeft,
            ),
            zone(
                "resize-bottom-right",
                ResizeEdge::BottomRight,
                point(right - band, bottom - band),
                corner,
                CursorStyle::ResizeUpLeftDownRight,
            ),
        ]
    }
}
