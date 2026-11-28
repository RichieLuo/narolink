use super::{Language, remote::RemoteClient, style::*};
use crate::engine::attachments::{HistoryImage, ImageAttachment, InlineImage};
use eframe::egui::{self, ColorImage, Rect, TextureHandle, Ui, Vec2, vec2};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

const TILE: Vec2 = vec2(88., 72.);
const MAX_THUMBNAILS: usize = 96;
const MAX_PREVIEW_EDGE: u32 = 2560;

pub(super) struct PromptImage<'a> {
    pub name: &'a str,
    pub source: Option<ThumbnailSource<'a>>,
}

pub(super) enum ThumbnailSource<'a> {
    Local(&'a Path),
    Inline(&'a InlineImage),
    Sent(ImageAttachment),
    Remote(&'a RemoteClient, RemoteImage<'a>),
}

#[derive(Clone, Copy)]
pub(super) enum RemoteImage<'a> {
    Run(&'a str, usize),
    Preview(&'a str),
}

#[derive(PartialEq, Eq)]
enum ImageKey {
    Local(PathBuf),
    Inline(uuid::Uuid),
    Sent(String),
    RemoteRun(String, String, usize),
    RemotePreview(String, String),
}

impl<'a> ThumbnailSource<'a> {
    pub(super) fn history(
        image: &'a HistoryImage,
        client: Option<&'a RemoteClient>,
    ) -> Option<Self> {
        match (image, client) {
            (HistoryImage::Remote(key), Some(client)) => {
                Some(Self::Remote(client, RemoteImage::Preview(key)))
            }
            (HistoryImage::Local(path), None) => Some(Self::Local(path)),
            (HistoryImage::Inline(image), None) => Some(Self::Inline(image)),
            _ => None,
        }
    }

    fn key(&self) -> ImageKey {
        match self {
            Self::Local(path) => ImageKey::Local(path.to_path_buf()),
            Self::Inline(image) => ImageKey::Inline(image.id()),
            Self::Sent(image) => ImageKey::Sent(image.id().to_owned()),
            Self::Remote(client, RemoteImage::Run(id, index)) => {
                ImageKey::RemoteRun(client.address().to_owned(), (*id).to_owned(), *index)
            }
            Self::Remote(client, RemoteImage::Preview(key)) => {
                ImageKey::RemotePreview(client.address().to_owned(), (*key).to_owned())
            }
        }
    }

    fn load(&self) -> Box<dyn FnOnce() -> Option<ImageAttachment> + Send> {
        match self {
            Self::Local(path) => {
                let path = path.to_path_buf();
                Box::new(move || {
                    // Only an explicit image from local conversation history can reach this path.
                    if !path.symlink_metadata().is_ok_and(|meta| meta.is_file()) {
                        return None;
                    }
                    ImageAttachment::from_file(path).ok()
                })
            }
            Self::Inline(image) => {
                let image = (*image).clone();
                Box::new(move || image.decode().ok())
            }
            Self::Sent(image) => {
                let image = image.clone();
                Box::new(move || Some(image))
            }
            Self::Remote(client, RemoteImage::Run(id, index)) => {
                let (client, id, index) = ((*client).clone(), (*id).to_owned(), *index);
                Box::new(move || {
                    ImageAttachment::from_bytes("image", &client.image(&id, index).ok()?).ok()
                })
            }
            Self::Remote(client, RemoteImage::Preview(key)) => {
                let (client, key) = ((*client).clone(), (*key).to_owned());
                Box::new(move || {
                    ImageAttachment::from_bytes("image", &client.preview(&key).ok()?).ok()
                })
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Thumbnails {
    cached: VecDeque<(ImageKey, Option<TextureHandle>)>,
    pending: Option<(ImageKey, Receiver<Option<ColorImage>>)>,
    preview: Option<ImagePreview>,
}

struct ImagePreview {
    name: String,
    state: PreviewState,
}

enum PreviewState {
    Loading(Receiver<Option<ColorImage>>),
    Ready(TextureHandle),
    Unavailable,
}

impl Thumbnails {
    pub fn is_preview_open(&self) -> bool {
        self.preview.is_some()
    }

    pub fn show_preview(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        let Some(preview) = &mut self.preview else {
            return;
        };
        if let PreviewState::Loading(inbox) = &preview.state
            && let Some(texture) = poll_texture(ctx, inbox)
        {
            preview.state = match texture {
                Some(texture) => PreviewState::Ready(texture),
                None => PreviewState::Unavailable,
            };
        }

        let viewport = ctx.content_rect();
        let max_width = (viewport.width() - 100.).max(1.);
        let max_height = (viewport.height() - 132.).max(1.);
        let modal = egui::Modal::new(egui::Id::new("sent image preview"))
            .frame(popup(palette, egui::Margin::same(16)))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                let language = Language::in_ui(ui);
                let image_size = match &preview.state {
                    PreviewState::Ready(texture) => {
                        let size = texture.size_vec2();
                        size * (max_width / size.x).min(max_height / size.y).min(1.)
                    }
                    _ => vec2(max_width.min(320.), 132.),
                };
                ui.set_width(image_size.x.max(max_width.min(250.)));
                ui.add(egui::Label::new(&preview.name).truncate())
                    .on_hover_text(&preview.name);
                ui.add_space(8.);
                ui.vertical_centered(|ui| match &preview.state {
                    PreviewState::Ready(texture) => {
                        ui.add(
                            egui::Image::new((texture.id(), image_size)).alt_text(&preview.name),
                        );
                    }
                    PreviewState::Loading(_) => {
                        ui.add_space(42.);
                        ui.add(egui::Spinner::new().size(24.).color(palette.accent));
                        ui.add_space(42.);
                    }
                    PreviewState::Unavailable => {
                        ui.add_space(42.);
                        ui.label(
                            language
                                .format("{name} · 暂不可预览", &[("name", preview.name.as_str())]),
                        );
                        ui.add_space(42.);
                    }
                });
                preview_close_button(ui, viewport, language.text("关闭")).clicked()
            });
        if modal.inner || modal.should_close() {
            self.preview = None;
        }
    }

    pub fn show(&mut self, ui: &mut Ui, images: &[PromptImage<'_>]) {
        let ready = self
            .pending
            .as_ref()
            .and_then(|(_, inbox)| poll_texture(ui.ctx(), inbox));
        if let Some(texture) = ready {
            let (key, _) = self.pending.take().unwrap();
            if self.cached.len() == MAX_THUMBNAILS {
                self.cached.pop_front();
            }
            self.cached.push_back((key, texture));
        }

        let gap = ui.spacing().item_spacing.x;
        let width = (ui.available_width() * 0.82).max(TILE.x);
        let columns = ((width + gap) / (TILE.x + gap)).floor() as usize;
        for row in images.chunks(columns) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                for image in row.iter().rev() {
                    self.tile(ui, image);
                }
            });
        }
    }

    fn tile(&mut self, ui: &mut Ui, image: &PromptImage<'_>) {
        let palette = Palette::of(ui);
        let (rect, response) = ui.allocate_exact_size(TILE, egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }

        let cached = image.source.as_ref().and_then(|source| {
            let key = source.key();
            if let Some(index) = self.cached.iter().position(|(cached, _)| cached == &key) {
                let entry = self.cached.remove(index).unwrap();
                let texture = entry.1.clone();
                self.cached.push_back(entry);
                Some(texture)
            } else {
                if self.pending.is_none() {
                    let size = [TILE.x as u32 * 2, TILE.y as u32 * 2];
                    self.pending = Some((key, load_image(ui.ctx(), source, size)));
                }
                None
            }
        });
        if let Some(Some(texture)) = &cached {
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let fitted = Rect::from_min_size(
                egui::pos2(
                    rect.right() - size.x * scale,
                    rect.center().y - size.y * scale / 2.,
                ),
                size * scale,
            );
            egui::Image::new((texture.id(), fitted.size()))
                .corner_radius(7)
                .paint_at(ui, fitted);
        } else {
            ui.painter().rect_filled(rect, 7., palette.soft);
            Icon::Image.paint(ui.painter(), rect.center(), palette.muted);
        }

        let language = Language::in_ui(ui);
        let (response, description, widget_type) = if let Some(source) = &image.source
            && !matches!(cached, Some(None))
        {
            let response = response
                .interact(egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                let max_side = ui.input(|input| input.max_texture_side as u32);
                let limit = max_side.min(MAX_PREVIEW_EDGE);
                self.preview = Some(ImagePreview {
                    name: image.name.to_owned(),
                    state: PreviewState::Loading(load_image(ui.ctx(), source, [limit, limit])),
                });
            }
            (
                response,
                language.format("{name} · 查看大图", &[("name", image.name)]),
                egui::WidgetType::Button,
            )
        } else {
            (
                response,
                language.format("{name} · 暂不可预览", &[("name", image.name)]),
                egui::WidgetType::Image,
            )
        };
        response.widget_info(|| egui::WidgetInfo::labeled(widget_type, true, &description));
        response.on_hover_text(description);
    }
}

fn preview_close_button(ui: &mut Ui, viewport: Rect, name: &str) -> egui::Response {
    let palette = Palette::of(ui);
    const SIZE: f32 = 44.;
    const INSET: f32 = 16.;
    let bounds = Rect::from_min_size(
        viewport.right_top() + vec2(-INSET - SIZE, INSET),
        Vec2::splat(SIZE),
    );
    // Keep the control on the modal layer without affecting the image panel's layout.
    let mut overlay = ui.new_child(egui::UiBuilder::new().max_rect(bounds));
    overlay.set_clip_rect(viewport);
    let response = overlay
        .interact(
            bounds,
            egui::Id::new("close sent image preview"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    let (fill, stroke) = if response.hovered() || response.has_focus() {
        (palette.tint, palette.tint_line)
    } else {
        (palette.paper, palette.line)
    };
    let painter = overlay.painter();
    painter.circle(
        bounds.center(),
        SIZE / 2.,
        fill,
        egui::Stroke::new(1., stroke),
    );
    Icon::Close.paint(painter, bounds.center(), palette.ink);
    response.on_hover_text(name)
}

fn poll_texture(
    ctx: &egui::Context,
    inbox: &Receiver<Option<ColorImage>>,
) -> Option<Option<TextureHandle>> {
    match inbox.try_recv() {
        Ok(image) => {
            Some(image.map(|image| ctx.load_texture("prompt image", image, Default::default())))
        }
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(None),
    }
}

fn load_image(
    ctx: &egui::Context,
    source: &ThumbnailSource<'_>,
    [width, height]: [u32; 2],
) -> Receiver<Option<ColorImage>> {
    let load = source.load();
    let ctx = ctx.clone();
    let (outbox, inbox) = mpsc::channel();
    std::thread::spawn(move || {
        let image = (|| {
            let image = load()?;
            let decoded =
                image::load_from_memory_with_format(image.png_bytes(), image::ImageFormat::Png)
                    .ok()?;
            let pixels = if decoded.width() > width || decoded.height() > height {
                decoded.thumbnail(width, height)
            } else {
                decoded
            }
            .into_rgba8();
            Some(ColorImage::from_rgba_unmultiplied(
                [pixels.width() as usize, pixels.height() as usize],
                pixels.as_raw(),
            ))
        })();
        let _ = outbox.send(image);
        ctx.request_repaint();
    });
    inbox
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2};
    use std::time::{Duration, Instant};

    fn click(position: Pos2) -> Vec<Event> {
        let mut events = vec![Event::PointerMoved(position)];
        events.extend([true, false].map(|pressed| Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }));
        events
    }

    #[test]
    fn preview_close_stays_in_the_window_corner_and_handles_clicks() {
        let ctx = egui::Context::default();
        let (_outbox, inbox) = mpsc::channel();
        let mut thumbnails = Thumbnails {
            preview: Some(ImagePreview {
                name: "photo.png".into(),
                state: PreviewState::Loading(inbox),
            }),
            ..Default::default()
        };
        let frame = |thumbnails: &mut Thumbnails, width, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 700.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    Language::English.apply(ctx);
                    thumbnails.show_preview(ctx);
                },
            )
        };
        let close_id = egui::Id::new("close sent image preview");
        frame(&mut thumbnails, 1000., vec![]);
        let expected = Rect::from_min_size(egui::pos2(940., 16.), vec2(44., 44.));
        assert_eq!(ctx.read_response(close_id).unwrap().rect, expected);
        for size in [[80, 56], [740, 490]] {
            let texture = ctx.load_texture(
                "test photo",
                ColorImage::filled(size, Palette::LIGHT.ink),
                Default::default(),
            );
            thumbnails.preview.as_mut().unwrap().state = PreviewState::Ready(texture);
            let rendered = frame(&mut thumbnails, 1000., vec![]);
            assert_eq!(ctx.read_response(close_id).unwrap().rect, expected);
            assert!(rendered.shapes.iter().any(|clipped| {
                matches!(&clipped.shape, egui::Shape::Circle(circle)
                    if circle.center == expected.center() && clipped.clip_rect.contains_rect(expected))
            }));
        }

        let rendered = frame(&mut thumbnails, 760., vec![]);
        let resized = Rect::from_min_size(egui::pos2(700., 16.), vec2(44., 44.));
        assert_eq!(ctx.content_rect().width(), 760.);
        assert!(rendered.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Circle(circle)
                if circle.center == resized.center())
        }));
        frame(&mut thumbnails, 760., vec![]);
        assert_eq!(ctx.read_response(close_id).unwrap().rect, resized);
        let clicked = frame(&mut thumbnails, 760., click(resized.center()));
        assert!(clicked.platform_output.events.iter().any(|event| {
            matches!(event, egui::output::OutputEvent::Clicked(info) if info.label.as_deref() == Some("Close"))
        }));
        assert!(!thumbnails.is_preview_open());
    }

    #[test]
    fn sent_thumbnail_opens_a_larger_image_and_escape_closes_it() {
        let sent =
            ImageAttachment::from_rgba("photo.png", 640, 320, &vec![255; 640 * 320 * 4]).unwrap();
        let images = [PromptImage {
            name: sent.name(),
            source: Some(ThumbnailSource::Sent(sent.clone())),
        }];
        let ctx = egui::Context::default();
        let mut thumbnails = Thumbnails::default();
        let mut position = Pos2::ZERO;
        let frame = |thumbnails: &mut Thumbnails, position: &mut Pos2, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000., 700.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        *position = ui.available_rect_before_wrap().right_top()
                            + vec2(-TILE.x / 2., TILE.y / 2.);
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            thumbnails.show(ui, &images);
                        });
                    });
                    thumbnails.show_preview(ctx);
                },
            )
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while thumbnails.cached.is_empty() {
            assert!(Instant::now() < deadline, "thumbnail did not load");
            frame(&mut thumbnails, &mut position, vec![]);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(thumbnails.cached[0].1.as_ref().unwrap().size(), [176, 88]);

        let click_position = position;
        frame(&mut thumbnails, &mut position, click(click_position));
        assert!(
            thumbnails.is_preview_open(),
            "thumbnail click did not open a preview"
        );

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let rendered = frame(&mut thumbnails, &mut position, vec![]);
            match &thumbnails.preview.as_ref().unwrap().state {
                PreviewState::Ready(texture) => {
                    assert_eq!(texture.size(), [640, 320]);
                    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000., 700.));
                    assert!(
                        ctx.tessellate(rendered.shapes, rendered.pixels_per_point)
                            .iter()
                            .any(|clipped| {
                                matches!(&clipped.primitive, egui::epaint::Primitive::Mesh(mesh)
                                    if mesh.texture_id == texture.id()
                                        && screen.intersect(clipped.clip_rect).contains_rect(mesh.calc_bounds()))
                            }),
                        "the full-size image was clipped or missing"
                    );
                    break;
                }
                PreviewState::Loading(_) => {
                    assert!(Instant::now() < deadline, "preview did not load");
                    std::thread::sleep(Duration::from_millis(1));
                }
                PreviewState::Unavailable => panic!("preview was unavailable"),
            }
        }

        frame(
            &mut thumbnails,
            &mut position,
            vec![Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert!(!thumbnails.is_preview_open());
    }
}
