//! An on-demand gallery backed by the archived originals, independent of source files.

use super::*;
use crate::plugins::clipboard::{Result, content::decode_image, history::SavedPicture};
use gpui_kit::component::spinner::Spinner;
use image::Frame;

pub(super) struct Preview {
    pub id: i64,
    count: usize,
    index: usize,
    has_paths: bool,
    pub focus: FocusHandle,
    pub picture: Option<std::result::Result<Picture, &'static str>>,
    load: Task<()>,
}

impl Preview {
    /// Retires the previous task and texture when the page, entry or entire view goes away.
    pub fn unload(&mut self, window: Option<&mut Window>, cx: &mut App) {
        self.load = Task::ready(());
        if let Some(Ok(picture)) = self.picture.take()
            && let Ok(image) = picture.image
        {
            cx.drop_image(image, window);
        }
    }
}

pub(super) struct Picture {
    position: i64,
    label: String,
    image: std::result::Result<Arc<RenderImage>, &'static str>,
}

impl Picture {
    /// Decodes one frame off-thread and bounds the texture before uploading it to the GPU.
    fn raster(bytes: &[u8]) -> Result<Arc<RenderImage>> {
        let (mut raster, orientation) = decode_image(bytes, image::guess_format(bytes)?)?;
        if raster.width().max(raster.height()) > 2048 {
            raster = raster.thumbnail(2048, 2048);
        }
        raster.apply_orientation(orientation);
        // GPUI's sprite atlas consumes BGRA pixels.
        let mut pixels = raster.into_rgba8();
        for pixel in pixels.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Ok(Arc::new(RenderImage::new([Frame::new(pixels)])))
    }
}

impl From<SavedPicture> for Picture {
    /// Keeps the original copy action available even when the display raster exceeds its budget.
    fn from(saved: SavedPicture) -> Self {
        let image = Self::raster(&saved.bytes).map_err(|err| {
            tracing::warn!("could not decode clipboard image preview: {}", err);
            err.key()
        });
        Self {
            position: saved.position,
            label: saved.label,
            image,
        }
    }
}

impl HistoryView {
    /// Opens the first archived image and routes gallery keys away from the search field.
    pub(super) fn open_preview(
        &mut self,
        id: i64,
        count: usize,
        has_paths: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(Some(id));
        self.copy_after_query = false;
        self.message = None;
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.preview = Some(Preview {
            id,
            count,
            index: 0,
            has_paths,
            focus,
            picture: None,
            load: Task::ready(()),
        });
        self.show_picture(0, window, cx);
    }

    /// Loads just the requested image and drops stale work before switching pages.
    fn show_picture(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = &mut self.preview else {
            return;
        };
        if index >= preview.count {
            return;
        }
        preview.unload(Some(window), cx);
        preview.index = index;
        self.message = None;
        let id = preview.id;
        let history = cx.global::<ClipboardState>().core.history.clone();
        let load =
            cx.background_spawn(async move { history.picture(id, index).map(Picture::from) });
        preview.load = cx.spawn(async move |this, cx| {
            let result: Result<Picture> = load.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(preview) = &mut this.preview {
                    preview.picture = Some(result.map_err(|err| {
                        tracing::warn!(
                            "could not preview clipboard entry {} image {}: {}",
                            id,
                            index + 1,
                            err
                        );
                        err.key()
                    }));
                    cx.notify();
                }
            });
        });
        cx.notify();
    }

    /// Restores the search focus, query and scroll position on leaving the gallery.
    fn close_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut preview) = self.preview.take() {
            preview.unload(Some(window), cx);
        }
        self.message = None;
        window.focus(&self.search.focus_handle(cx), cx);
        cx.notify();
    }

    /// Fills the former list area with one picture and its position in the group.
    pub(super) fn render_preview(
        &self,
        preview: &Preview,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = preview.id;
        let index = preview.index;
        let count = preview.count;
        let picture = preview
            .picture
            .as_ref()
            .and_then(|result| result.as_ref().ok());
        let raster = preview.picture.as_ref().map(|page| match page {
            Ok(picture) => picture.image.as_ref(),
            Err(key) => Err(key),
        });
        let body = match raster {
            Some(Ok(image)) => img(Arc::clone(image))
                .id("clip-picture")
                .size_full()
                .object_fit(ObjectFit::ScaleDown)
                .into_any_element(),
            Some(Err(key)) => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!(*key).to_string())
                .into_any_element(),
            None => Spinner::new().into_any_element(),
        };
        v_flex()
            .size_full()
            .min_h_0()
            .track_focus(&preview.focus)
            .key_context(PREVIEW_CONTEXT)
            .on_action(
                cx.listener(|this, _: &ClosePreview, window, cx| this.close_preview(window, cx)),
            )
            .on_action(cx.listener(move |this, _: &PreviousImage, window, cx| {
                if let Some(previous) = index.checked_sub(1) {
                    this.show_picture(previous, window, cx);
                }
            }))
            .on_action(cx.listener(move |this, _: &NextImage, window, cx| {
                this.show_picture(index + 1, window, cx)
            }))
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .child(
                        Toolbar::button("close-clip-preview", IconName::ArrowLeft)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_preview(window, cx)),
                            )
                            .with_key_tooltip(
                                t!("clipboard.back_to_history"),
                                Some((&ClosePreview, PREVIEW_CONTEXT)),
                            ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(picture.map(|p| p.label.clone()).unwrap_or_default()),
                    )
                    .when(count > 1, |d| {
                        d.child(
                            h_flex()
                                .flex_none()
                                .gap_1()
                                .child(
                                    Toolbar::button("clip-previous-image", IconName::ChevronLeft)
                                        .disabled(index == 0)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Some(previous) = index.checked_sub(1) {
                                                this.show_picture(previous, window, cx);
                                            }
                                        }))
                                        .with_key_tooltip(
                                            t!("clipboard.previous_image"),
                                            Some((&PreviousImage, PREVIEW_CONTEXT)),
                                        ),
                                )
                                .child(div().text_sm().child(format!("{} / {}", index + 1, count)))
                                .child(
                                    Toolbar::button("clip-next-image", IconName::ChevronRight)
                                        .disabled(index + 1 >= count)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.show_picture(index + 1, window, cx)
                                        }))
                                        .with_key_tooltip(
                                            t!("clipboard.next_image"),
                                            Some((&NextImage, PREVIEW_CONTEXT)),
                                        ),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .p_2()
                    .child(body),
            )
            .child(
                Toolbar::default()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .children(self.message.clone()),
                    )
                    .when(preview.has_paths, |bar| {
                        bar.child(
                            Button::new("copy-clip-paths")
                                .small()
                                .label(t!("clipboard.copy_original_paths"))
                                .disabled(self.busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy(id, Selection::Paths, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("copy-preview-image")
                            .small()
                            .label(t!("clipboard.copy_image"))
                            .disabled(self.busy || picture.is_none())
                            .on_click({
                                let position = picture.map(|p| p.position);
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(position) = position {
                                        this.copy(id, Selection::One(position), cx);
                                    }
                                })
                            }),
                    ),
            )
    }
}
