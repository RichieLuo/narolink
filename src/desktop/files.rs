//! Desktop UI for read-only file previews.
use super::{
    Language, markdown,
    style::{Icon, Palette, lerp_color},
};
use crate::file_preview::{LoadResult, MAX_MARKDOWN_BYTES, TextFile, read_file};
use eframe::egui::{
    self, Align, Align2, Button, FontId, Label, Layout, Margin, Rect, RichText, Stroke, TextEdit,
    Ui, pos2, vec2,
};
use egui_commonmark::CommonMarkCache;
use egui_extras::syntax_highlighting::{self, CodeTheme};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

const MAX_HIGHLIGHT_BYTES: usize = 128 * 1024;
const CODE_FONT_SIZE: f32 = 13.;
const CODE_LINE_HEIGHT: f32 = 19.;

#[derive(Default)]
pub(super) struct FileViewer {
    preview: Option<Preview>,
}

struct Preview {
    path: PathBuf,
    workspace: PathBuf,
    inbox: Receiver<LoadResult>,
    content: Option<LoadResult>,
    raw: bool,
    cache: CommonMarkCache,
}

impl FileViewer {
    pub(super) fn is_open(&self) -> bool {
        self.preview.is_some()
    }

    pub(super) fn close(&mut self) {
        self.preview = None;
    }

    pub(super) fn open(&mut self, ctx: &egui::Context, path: PathBuf, workspace: &Path) {
        self.preview = Some(Preview::new(ctx, path, workspace.to_owned()));
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        let Some(preview) = &mut self.preview else {
            return;
        };
        preview.poll();
        let available = ctx.available_rect().width();
        let chat_width = (available * 0.46).clamp(360., 440.);
        let max_width = (available - chat_width).max(300.);
        let mut close = false;
        let mut refresh = false;
        let mut navigate = None;
        egui::SidePanel::right("file viewer split")
            .default_width((available * 0.54).clamp(300., 820.))
            .width_range(300. ..=max_width)
            .resizable(true)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(palette.paper))
            .show(ctx, |ui| {
                let language = Language::in_ui(ui);
                let bounds = ui.max_rect();
                let file = preview
                    .content
                    .as_ref()
                    .and_then(|result| result.as_ref().ok());
                let can_render =
                    file.is_some_and(|file| file.markdown && file.text.len() <= MAX_MARKDOWN_BYTES);
                ui.spacing_mut().item_spacing.y = 0.;
                egui::Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.x = 2.;
                        ui.spacing_mut().button_padding.x = 4.;
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 29.),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                close = Icon::Close
                                    .button(ui, language.text("关闭文件预览"), palette.muted)
                                    .clicked();
                                refresh = Icon::Refresh
                                    .button(ui, language.text("刷新文件"), palette.muted)
                                    .clicked();
                                if Icon::Folder
                                    .button(ui, language.text("在文件夹中显示"), palette.muted)
                                    .clicked()
                                {
                                    let path = preview.path.clone();
                                    std::thread::spawn(move || markdown::reveal_file(&path));
                                }
                                if let Some(file) = file
                                    && Icon::Copy
                                        .button(ui, language.text("复制文件内容"), palette.muted)
                                        .clicked()
                                {
                                    ctx.copy_text(file.text.clone());
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 7.;
                                    let icon = if file.is_some_and(|file| file.markdown) {
                                        Icon::Document
                                    } else {
                                        Icon::Code
                                    };
                                    let (rect, _) = ui
                                        .allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
                                    icon.paint(ui.painter(), rect.center(), palette.accent);
                                    let name = preview
                                        .path
                                        .file_name()
                                        .unwrap_or(preview.path.as_os_str())
                                        .to_string_lossy();
                                    ui.add(
                                        Label::new(
                                            RichText::new(name)
                                                .size(13.5)
                                                .color(palette.ink)
                                                .strong(),
                                        )
                                        .truncate(),
                                    );
                                });
                            },
                        );
                    });
                let directory = preview.path.parent().unwrap_or(&preview.path);
                let relative_directory = directory
                    .strip_prefix(&preview.workspace)
                    .ok()
                    .and_then(|_| preview.workspace.parent())
                    .and_then(|parent| directory.strip_prefix(parent).ok())
                    .unwrap_or(directory);
                let details = egui::Frame::new()
                    .inner_margin(Margin::symmetric(14, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 22.),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                if can_render {
                                    ui.spacing_mut().item_spacing.x = 3.;
                                    ui.spacing_mut().button_padding = vec2(7., 2.);
                                    for (raw, label) in [(true, "原始内容"), (false, "预览")]
                                    {
                                        let selected = preview.raw == raw;
                                        let button = Button::new(
                                            RichText::new(language.text(label)).size(11.5).color(
                                                if selected { palette.ink } else { palette.muted },
                                            ),
                                        )
                                        .small()
                                        .selected(selected)
                                        .frame_when_inactive(selected);
                                        if ui.add(button).clicked() {
                                            preview.raw = raw;
                                        }
                                    }
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add(
                                        Label::new(
                                            RichText::new(relative_directory.to_string_lossy())
                                                .size(11.5)
                                                .color(palette.muted),
                                        )
                                        .truncate()
                                        .show_tooltip_when_elided(false),
                                    )
                                    .on_hover_text(preview.path.to_string_lossy());
                                });
                            },
                        );
                    });
                ui.painter().hline(
                    bounds.x_range(),
                    details.response.rect.bottom() - 0.5,
                    Stroke::new(1., palette.line),
                );
                match &preview.content {
                    None => {
                        egui::Frame::new()
                            .inner_margin(Margin::same(16))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(language.text("正在读取文件…"));
                                });
                            });
                    }
                    Some(Err(error)) => {
                        egui::Frame::new()
                            .inner_margin(Margin::same(16))
                            .show(ui, |ui| {
                                ui.label(RichText::new(language.text(error)).color(palette.warn));
                            });
                    }
                    Some(Ok(file)) => {
                        if file.markdown && !can_render {
                            egui::Frame::new()
                                .fill(palette.canvas)
                                .inner_margin(Margin::symmetric(14, 7))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        RichText::new(language.text("文件较大，仅显示原始内容。"))
                                            .size(12.)
                                            .color(palette.muted),
                                    );
                                });
                        }
                        let raw = !can_render || preview.raw;
                        ui.push_id((&preview.path, raw), |ui| {
                            ui.set_clip_rect(
                                ui.clip_rect().intersect(ui.available_rect_before_wrap()),
                            );
                            if raw {
                                render_source(ui, file);
                            } else {
                                egui::ScrollArea::both().auto_shrink([false, false]).show(
                                    ui,
                                    |ui| {
                                        ui.spacing_mut().item_spacing.y = 7.;
                                        egui::Frame::new()
                                            .inner_margin(Margin::symmetric(24, 17))
                                            .show(ui, |ui| {
                                                navigate = markdown::render(
                                                    ui,
                                                    &mut preview.cache,
                                                    file.text.trim_start_matches('\u{feff}'),
                                                    directory,
                                                );
                                            });
                                    },
                                );
                            }
                        });
                    }
                }
                ui.painter().vline(
                    bounds.left() + 0.5,
                    bounds.y_range(),
                    Stroke::new(1., palette.line),
                );
            });
        if close {
            self.preview = None;
        } else if let Some(path) = navigate {
            self.preview = Some(Preview::new(ctx, path, preview.workspace.clone()));
        } else if refresh {
            // Replacing the receiver discards any stale result from the previous load.
            let mut next = Preview::new(ctx, preview.path.clone(), preview.workspace.clone());
            next.raw = preview.raw;
            self.preview = Some(next);
        }
    }
}

impl Preview {
    fn new(ctx: &egui::Context, path: PathBuf, workspace: PathBuf) -> Self {
        let (outbox, inbox) = mpsc::channel();
        let source = path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = outbox.send(read_file(&source));
            ctx.request_repaint();
        });
        Self {
            path,
            workspace,
            inbox,
            content: None,
            raw: false,
            cache: CommonMarkCache::default(),
        }
    }

    fn poll(&mut self) {
        match self.inbox.try_recv() {
            Ok(result) => self.content = Some(result),
            Err(mpsc::TryRecvError::Disconnected) if self.content.is_none() => {
                self.content = Some(Err("无法读取文件，请刷新重试。"));
            }
            _ => {}
        }
    }
}

fn render_source(ui: &mut Ui, file: &TextFile) -> egui::Response {
    let palette = Palette::of(ui);
    let gutter_left = ui.available_rect_before_wrap().left();
    let gutter_width = ((file.lines.ilog10() + 1).max(2) * 8 + 24) as i8;
    let gutter_right = gutter_left + f32::from(gutter_width);
    egui::ScrollArea::both()
        .id_salt("source viewport")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let clip = ui.clip_rect();
            ui.set_clip_rect(clip.with_min_x(gutter_right));
            let font = FontId::monospace(CODE_FONT_SIZE);
            let theme = CodeTheme::from_style(ui.style());
            let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _: f32| {
                let mut job = if file.text.len() <= MAX_HIGHLIGHT_BYTES && !file.text.is_empty() {
                    syntax_highlighting::highlight(
                        ui.ctx(),
                        ui.style(),
                        &theme,
                        buffer.as_str(),
                        &file.syntax,
                    )
                } else {
                    egui::text::LayoutJob::simple(
                        buffer.as_str().to_owned(),
                        font.clone(),
                        palette.ink,
                        f32::INFINITY,
                    )
                };
                job.first_row_min_height = CODE_LINE_HEIGHT;
                for section in &mut job.sections {
                    section.format.line_height = Some(CODE_LINE_HEIGHT);
                    section.format.color = lerp_color(section.format.color, palette.ink, 0.28);
                    if !section.format.underline.is_empty() {
                        section.format.underline.color = section.format.color;
                    }
                }
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };
            // The immutable buffer keeps text selection and copying available.
            let mut text = file.text.as_str();
            let output = TextEdit::multiline(&mut text)
                .id_salt("file source")
                .code_editor()
                .lock_focus(false)
                .frame(false)
                .font(font.clone())
                .margin(Margin {
                    left: gutter_width + 14,
                    right: 16,
                    top: 13,
                    bottom: 13,
                })
                .desired_width(ui.available_width())
                .desired_rows(1)
                .layouter(&mut layouter)
                .show(ui);
            ui.set_clip_rect(clip);
            let gutter = Rect::from_x_y_ranges(gutter_left..=gutter_right, clip.y_range());
            let painter = ui.painter().with_clip_rect(clip.intersect(gutter));
            painter.rect_filled(gutter, 0., palette.canvas);
            painter.vline(
                gutter_right - 0.5,
                gutter.y_range(),
                Stroke::new(1., palette.line),
            );

            // Use the text layout's row positions so scrolling, blank lines, and CJK stay aligned.
            let top = clip.top() - output.galley_pos.y;
            let bottom = clip.bottom() - output.galley_pos.y;
            let rows = &output.galley.rows;
            let first = rows.partition_point(|row| row.rect().bottom() <= top);
            for (line, row) in rows
                .iter()
                .enumerate()
                .skip(first)
                .take_while(|(_, row)| row.pos.y < bottom)
            {
                painter.text(
                    pos2(
                        gutter_right - 10.,
                        output.galley_pos.y + row.rect().center().y,
                    ),
                    Align2::RIGHT_CENTER,
                    (line + 1).to_string(),
                    FontId::monospace(12.),
                    palette.muted,
                );
            }
            output.response
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_preview::tests::TestDirectory;
    use egui::{Event, Pos2, Rect, vec2};

    fn loaded_viewer(path: PathBuf, result: LoadResult) -> FileViewer {
        let (_, inbox) = mpsc::channel();
        FileViewer {
            preview: Some(Preview {
                path,
                workspace: PathBuf::new(),
                inbox,
                content: Some(result),
                raw: false,
                cache: CommonMarkCache::default(),
            }),
        }
    }

    fn frame(ctx: &egui::Context, viewer: &mut FileViewer, events: Vec<Event>) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000., 700.))),
                events,
                ..Default::default()
            },
            |ctx| {
                Language::English.apply(ctx);
                viewer.show(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Chat stays visible");
                });
            },
        )
    }

    fn text_position(output: &egui::FullOutput, label: &str) -> Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() / 2.)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {label:?}"))
    }

    fn click(position: Pos2) -> Vec<Event> {
        vec![
            Event::PointerMoved(position),
            Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn markdown_preview_toggles_source_and_follows_relative_links() {
        for (source, target) in [
            (
                "# Preview title\n\n**bold text**\n\n[Next file](other.md)\n",
                "other.md",
            ),
            (
                "# Preview title\n\n> | Link | Status |\n> | --- | --- |\n> | [Next file][doc] | Pending |\n\n[doc]: a&amp;b.md",
                "a&b.md",
            ),
        ] {
            check_markdown_preview_navigation(source, target);
        }
    }

    fn check_markdown_preview_navigation(source: &str, target: &str) {
        let dir = TestDirectory::new();
        let path = dir.file("notes.md", source);
        let mut viewer = loaded_viewer(path.clone(), read_file(&path));
        let ctx = egui::Context::default();
        let _ = frame(&ctx, &mut viewer, vec![]);
        let rendered = frame(&ctx, &mut viewer, vec![]);
        text_position(&rendered, "Preview title");
        text_position(&rendered, "Chat stays visible");
        let position = text_position(&rendered, "Source");
        let _ = frame(&ctx, &mut viewer, click(position));
        assert!(viewer.preview.as_ref().unwrap().raw);
        let raw = frame(&ctx, &mut viewer, vec![]);
        assert!(raw.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == source)
        ));
        let _ = frame(&ctx, &mut viewer, click(text_position(&raw, "Preview")));
        assert!(!viewer.preview.as_ref().unwrap().raw);
        let rendered = frame(&ctx, &mut viewer, vec![]);
        let next = text_position(&rendered, "Next file");
        let _ = frame(&ctx, &mut viewer, click(next));
        assert_eq!(viewer.preview.as_ref().unwrap().path, dir.0.join(target));
        // A missing target still stays inside Naro with an error, never an OS open.
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }

    #[test]
    fn source_buffer_is_read_only_and_copy_keeps_markdown_syntax() {
        let ctx = egui::Context::default();
        let source = "# Title\n\n**unchanged**\n";
        let file = TextFile {
            text: source.into(),
            markdown: true,
            syntax: "md".into(),
            lines: 4,
        };
        let mut id = None;
        {
            let mut draw = |events: Vec<Event>| {
                ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            id = Some(render_source(ui, &file).id);
                        });
                    },
                )
            };
            let initial = draw(vec![]);
            let position = text_position(&initial, source);
            let _ = draw(click(position));
            let changed = draw(vec![
                Event::Text("do not write".into()),
                Event::Paste("or paste".into()),
            ]);
            assert_eq!(file.text, source);
            assert!(changed.shapes.iter().any(
                |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == source)
            ));
        }
        ctx.memory_mut(|memory| memory.request_focus(id.unwrap()));
        let _ = ctx.run(
            egui::RawInput {
                events: vec![Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                modifiers: egui::Modifiers::COMMAND,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| render_source(ui, &file));
            },
        );
        let copied = ctx.run(
            egui::RawInput {
                events: vec![Event::Copy],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| render_source(ui, &file));
            },
        );
        assert!(
            copied.platform_output.commands.iter().any(
                |command| matches!(command, egui::OutputCommand::CopyText(text) if text == source)
            ),
            "copy output: {:?}; focus: {:?}; source id: {:?}; cursor: {:?}",
            copied.platform_output.commands,
            ctx.memory(|m| m.focused()),
            id,
            TextEdit::load_state(&ctx, id.unwrap()).map(|s| s.cursor.char_range())
        );
    }

    #[test]
    fn source_gutter_aligns_with_rows_and_long_code_keeps_chat_visible() {
        let ctx = egui::Context::default();
        super::super::style::setup(&ctx);
        let source = format!("// {}\r\n\r\nfn main() {{}}\n", "long line ".repeat(100));
        let dir = TestDirectory::new();
        let path = dir.file("main.rs", &source);
        let mut viewer = loaded_viewer(path.clone(), read_file(&path));
        let _ = frame(&ctx, &mut viewer, vec![]);
        let output = frame(&ctx, &mut viewer, vec![]);
        let chat = text_position(&output, "Chat stays visible");
        let first = text_position(&output, "1");
        let last = text_position(&output, "4");
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "1")
                && shape.clip_rect.contains(first)
        }));
        let code = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == source => Some(text),
                _ => None,
            })
            .unwrap();
        assert_eq!(code.galley.rows.len(), 4);
        assert!(chat.x < first.x && first.x < code.pos.x);
        assert!((first.y - code.pos.y - code.galley.rows[0].rect().center().y).abs() < 2.);
        assert!((last.y - code.pos.y - code.galley.rows[3].rect().center().y).abs() < 2.);
    }

    #[test]
    fn large_markdown_uses_source_and_errors_are_translated() {
        let ctx = egui::Context::default();
        let mut viewer = loaded_viewer(
            PathBuf::from("large.md"),
            Ok(TextFile {
                text: "# Large\n".repeat(MAX_MARKDOWN_BYTES / 8 + 1),
                markdown: true,
                syntax: "md".into(),
                lines: MAX_MARKDOWN_BYTES / 8 + 2,
            }),
        );
        let output = frame(&ctx, &mut viewer, vec![]);
        text_position(&output, "This file is large. Showing source only.");
        viewer = loaded_viewer(PathBuf::from("missing.md"), Err("文件不存在或已被移动。"));
        let output = frame(&ctx, &mut viewer, vec![]);
        text_position(&output, "The file does not exist or has been moved.");
    }

    #[test]
    fn replacing_or_closing_preview_discards_old_background_results() {
        let ctx = egui::Context::default();
        let dir = TestDirectory::new();
        let old = dir.file("old.md", "# Old");
        let next = dir.file("next.txt", "new source");
        let mut viewer = FileViewer::default();
        viewer.open(&ctx, old, &dir.0);
        viewer.open(&ctx, next.clone(), &dir.0);
        let preview = viewer.preview.as_mut().unwrap();
        let result = preview
            .inbox
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        preview.content = Some(result);
        preview.poll();
        assert_eq!(preview.path, next);
        assert_eq!(
            preview.content.as_ref().unwrap().as_ref().unwrap().text,
            "new source"
        );
        viewer.preview = None;
        assert!(!viewer.is_open());
        frame(&ctx, &mut viewer, vec![]);
    }
}
