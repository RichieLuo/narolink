use super::*;
use crate::engine::Message;
use eframe::egui::{
    Align, Color32, FontFamily, FontId, Layout, Margin, RichText, Stroke, TextStyle,
};
use egui_commonmark::CommonMarkViewer;
use pulldown_cmark::{Event, Options, Parser, Tag};

const CANVAS: Color32 = Color32::from_rgb(245, 246, 251);
const PAPER: Color32 = Color32::WHITE;
const INK: Color32 = Color32::from_rgb(30, 37, 53);
const MUTED: Color32 = Color32::from_rgb(96, 106, 129);
const LINE: Color32 = Color32::from_rgb(221, 225, 237);
const ACCENT: Color32 = Color32::from_rgb(72, 78, 201);
const RAIL: Color32 = Color32::from_rgb(32, 37, 54);
const RAIL_CARD: Color32 = Color32::from_rgb(48, 55, 77);
const RAIL_TEXT: Color32 = Color32::from_rgb(240, 243, 255);
const RAIL_DIM: Color32 = Color32::from_rgb(183, 193, 219);
const WARN: Color32 = Color32::from_rgb(139, 78, 44);

pub(super) fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font_name = "noto-cjk".to_owned();
    fonts.font_data.insert(
        font_name.clone(),
        egui::FontData::from_static(include_bytes!("../../assets/NotoSansCJKsc-Regular.otf"))
            .into(),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(font_name.clone());
    }
    ctx.set_fonts(fonts);
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = CANVAS;
    visuals.window_fill = PAPER;
    visuals.extreme_bg_color = PAPER;
    visuals.selection.bg_fill = Color32::from_rgb(226, 230, 254);
    visuals.selection.stroke = Stroke::new(1., INK);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(241, 242, 250);
    visuals.widgets.inactive.weak_bg_fill = visuals.widgets.inactive.bg_fill;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1., LINE);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(229, 233, 255);
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8., 8.);
        style.spacing.button_padding = egui::vec2(11., 6.);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(15.));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(14.));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.));
    });
}

impl Desktop {
    pub(super) fn paint(&mut self, ctx: &egui::Context) {
        self.sidebar(ctx);
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(PAPER)
                    .inner_margin(Margin::symmetric(22, 12))
                    .stroke(Stroke::new(1., LINE)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        egui::vec2((width - 195.).max(100.), 44.),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(self.view.title.as_deref().unwrap_or("新建会话"))
                                        .size(17.)
                                        .strong(),
                                )
                                .truncate(),
                            );
                            if let Some(session) = &self.view.session {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(session.cwd.display().to_string())
                                            .color(MUTED)
                                            .monospace()
                                            .size(12.),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(session.cwd.display().to_string());
                            }
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("手机连接").clicked() {
                            self.pair_dialog = true;
                        }
                        ui.label(
                            RichText::new(self.view.provider.label())
                                .color(ACCENT)
                                .strong(),
                        );
                    });
                });
            });
        egui::TopBottomPanel::bottom("composer")
            .frame(
                egui::Frame::new()
                    .fill(CANVAS)
                    .inner_margin(Margin::symmetric(20, 12)),
            )
            .show(ctx, |ui| centered(ui, 780., |ui| self.composer(ui, ctx)));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("chat", self.epoch))
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(16.);
                        centered(ui, 760., |ui| self.conversation(ui, ctx));
                        ui.add_space(22.);
                    });
            });
        self.phone(ctx);
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(270.)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(RAIL)
                    .inner_margin(Margin::symmetric(14, 14)),
            )
            .show(ctx, |ui| {
                let visuals = &mut ui.style_mut().visuals;
                visuals.override_text_color = Some(RAIL_TEXT);
                visuals.widgets.inactive.weak_bg_fill = RAIL_CARD;
                visuals.widgets.inactive.bg_fill = RAIL_CARD;
                visuals.widgets.inactive.bg_stroke =
                    Stroke::new(1., Color32::from_rgb(76, 86, 117));
                visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(60, 71, 112);
                visuals.widgets.hovered.bg_fill = visuals.widgets.hovered.weak_bg_fill;
                visuals.widgets.hovered.fg_stroke = Stroke::new(1., RAIL_TEXT);
                visuals.extreme_bg_color = Color32::from_rgb(24, 30, 44);
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(31., 31.), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 7, ACCENT);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "N",
                        FontId::proportional(19.),
                        PAPER,
                    );
                    ui.painter().circle_filled(
                        rect.right_top() + egui::vec2(-6., 7.),
                        1.8,
                        Color32::from_rgb(183, 196, 255),
                    );
                    ui.label(RichText::new("NaroLink").size(22.).strong());
                });
                ui.add_space(10.);
                if ui
                    .add_sized(
                        [ui.available_width(), 38.],
                        egui::Button::new("＋ 新建会话"),
                    )
                    .clicked()
                {
                    self.new_view(ctx);
                }
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("会话").color(RAIL_DIM).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("刷新").clicked() {
                            self.refresh_inventory(ctx, true);
                        }
                    });
                });
                ui.add_sized(
                    [ui.available_width(), 31.],
                    egui::TextEdit::singleline(&mut self.search).hint_text("搜索会话或目录"),
                );
                ui.horizontal_wrapped(|ui| {
                    for (provider, label) in [
                        (None, "全部"),
                        (Some(Provider::Codex), "Codex"),
                        (Some(Provider::Claude), "Claude"),
                        (Some(Provider::Antigravity), "Antigravity"),
                    ] {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(label).size(12.))
                                    .selected(self.filters == provider)
                                    .small(),
                            )
                            .clicked()
                        {
                            self.filters = provider;
                        }
                    }
                });
                ui.separator();
                self.session_list(ui, ctx);
            });
    }

    fn session_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(warning) = Provider::ALL
            .into_iter()
            .filter(|provider| self.filters.is_none_or(|filter| filter == *provider))
            .find_map(|provider| {
                self.inventories
                    .get(&provider)
                    .and_then(|result| match result {
                        Ok(inventory) => inventory.warning.clone(),
                        Err(error) => Some(format!("{} · {error}", provider.label())),
                    })
            })
        {
            ui.add(egui::Label::new(RichText::new(warning).color(RAIL_DIM).size(12.)).wrap());
        }
        let query = self.search.trim().to_lowercase();
        let mut sessions: Vec<_> = self
            .inventories
            .iter()
            .filter(|(provider, _)| self.filters.is_none_or(|filter| filter == **provider))
            .filter_map(|(_, list)| list.as_ref().ok())
            .flat_map(|list| &list.sessions)
            .filter(|row| {
                query.is_empty()
                    || row.title.to_lowercase().contains(&query)
                    || row.cwd.to_string_lossy().to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        sessions.sort_by_key(|row| std::cmp::Reverse(row.updated));
        let running = self
            .running
            .iter()
            .filter(|row| self.filters.is_none_or(|filter| filter == row.provider))
            .cloned()
            .collect::<Vec<_>>();
        egui::ScrollArea::vertical()
            .id_salt("session rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for run in running {
                    if sidebar_row(
                        ui,
                        &run.title,
                        &format!("正在运行 · {}", run.provider.label()),
                        self.view.run.as_ref() == Some(&run.id),
                    ) {
                        self.open_run(ctx, run);
                    }
                }
                if sessions.is_empty() {
                    let text = if !self.inventory_pending.is_empty() {
                        "正在读取本机历史…"
                    } else if !query.is_empty() {
                        "没有匹配的会话"
                    } else {
                        "暂无本机会话"
                    };
                    ui.label(RichText::new(text).color(RAIL_DIM).size(13.));
                }
                for row in sessions {
                    let selected = self.view.provider == row.provider
                        && self.view.id.as_ref() == Some(&row.id);
                    let folder = row.cwd.file_name().unwrap_or_default().to_string_lossy();
                    if sidebar_row(
                        ui,
                        &row.title,
                        &format!("{} · {folder}", row.provider.label()),
                        selected,
                    ) {
                        self.open(ctx, row);
                    }
                }
            });
    }

    fn conversation(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.view.id.is_none() && self.view.run.is_none() {
            card(PAPER).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("引擎").size(16.).strong());
                ui.add_space(6.);
                ui.add_enabled_ui(!self.view.pending, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for provider in Provider::ALL {
                            if ui
                                .add(
                                    egui::Button::new(provider.label())
                                        .selected(self.view.provider == provider),
                                )
                                .clicked()
                            {
                                self.view.provider = provider;
                                self.view.model = None;
                                self.load_models(ctx, provider, false);
                            }
                        }
                    });
                    ui.add_space(8.);
                    ui.horizontal(|ui| {
                        if ui.button("选择工作目录…").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .set_title("选择 NaroLink 的工作目录")
                                .pick_folder()
                        {
                            self.folder = path;
                        }
                        if self.folder.as_os_str().is_empty() {
                            ui.label(RichText::new("请选择一个文件夹").color(MUTED));
                        } else {
                            let path = self.folder.display().to_string();
                            ui.add(egui::Label::new(RichText::new(&path).monospace()).truncate())
                                .on_hover_text(path);
                        }
                    });
                });
            });
        }
        if let Some(warning) = &self.view.transcript.warning {
            ui.label(RichText::new(warning).color(WARN));
            if self.view.transcript.messages.is_empty()
                && let Some(session) = &self.view.session
                && let Some(preview) = &session.preview
            {
                ui.label(RichText::new(preview).color(MUTED));
            }
        }
        let provider = self.view.provider;
        let shown = self.view.transcript.messages.len().saturating_sub(300);
        if shown > 0 {
            ui.label(RichText::new("显示本会话最近 300 条消息。").color(MUTED));
        }
        for entry in &self.view.transcript.messages[shown..] {
            render_message(ui, &mut self.markdown, entry, provider);
        }
        self.render_live(ui, provider);
    }
}

impl Desktop {
    fn render_live(&mut self, ui: &mut egui::Ui, provider: Provider) {
        let Some(snapshot) = &self.view.snapshot else {
            return;
        };
        render_message(
            ui,
            &mut self.markdown,
            &Message {
                role: Role::User,
                text: snapshot.prompt.clone(),
            },
            provider,
        );
        if !snapshot.images.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for image in &snapshot.images {
                    ui.label(RichText::new(image).color(ACCENT).size(13.));
                }
            });
        }
        if !snapshot.output.is_empty() {
            render_message(
                ui,
                &mut self.markdown,
                &Message {
                    role: Role::Assistant,
                    text: snapshot.output.clone(),
                },
                provider,
            );
        }
        if snapshot.status == Status::Running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(&snapshot.activity).color(MUTED));
            });
        } else if snapshot.status == Status::Stopped {
            ui.label(RichText::new("已停止").color(MUTED));
        }
        if let Some(error) = &snapshot.error {
            ui.label(RichText::new(error).color(WARN));
        }
        if let Some(approval) = &snapshot.approval {
            card(Color32::from_rgb(255, 249, 234)).show(ui, |ui| {
                ui.label(RichText::new(format!("确认操作 · {}", approval.title)).strong());
                egui::ScrollArea::vertical()
                    .id_salt(&approval.id)
                    .max_height(150.)
                    .show(ui, |ui| {
                        ui.label(RichText::new(&approval.details).monospace());
                    });
                ui.horizontal(|ui| {
                    for (approved, label) in [(true, "允许一次"), (false, "拒绝")] {
                        if ui.button(label).clicked() {
                            let result = self
                                .service
                                .run(&snapshot.run.id)
                                .and_then(|run| run.approve(&approval.id, approved));
                            self.note = result.err();
                        }
                    }
                });
            });
        }
    }

    fn composer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        composer_frame().show(ui, |ui| {
            let busy = self.view.busy();
            self.draft_images(ui);
            let rows = self.view.draft.lines().count().clamp(1, 4);
            let editor_id = ui.make_persistent_id("prompt");
            ui.add_enabled(
                !busy,
                egui::TextEdit::multiline(&mut self.view.draft)
                    .id(editor_id)
                    .hint_text("你想做什么？")
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY)
                    .frame(false),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy && self.view.attachments.len() < MAX_IMAGE_ATTACHMENTS,
                        egui::Button::new("图片…").frame(false),
                    )
                    .clicked()
                    && let Some(paths) = rfd::FileDialog::new()
                        .set_title("添加图片")
                        .add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp"])
                        .pick_files()
                {
                    self.add_images(paths);
                }
                let model_result = self.models.get(&self.view.provider);
                let selected = self.view.model.as_deref();
                let model_name = selected
                    .and_then(|id| {
                        model_result
                            .and_then(|result| result.as_ref().ok())
                            .and_then(|models| {
                                models
                                    .iter()
                                    .find(|model| model.id == id)
                                    .map(|model| model.label.as_str())
                            })
                    })
                    .or(selected)
                    .unwrap_or(if self.view.id.is_some() {
                        "会话当前模型"
                    } else {
                        "默认模型"
                    })
                    .to_owned();
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_id_salt("models")
                        .selected_text(model_name)
                        .width(144.)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.view.model, None, "使用当前模型");
                            if let Some(Ok(models)) = model_result {
                                for model in models {
                                    ui.selectable_value(
                                        &mut self.view.model,
                                        Some(model.id.clone()),
                                        &model.label,
                                    );
                                }
                            }
                        });
                });
                let refresh = ui.add_enabled(
                    !self.model_pending.contains(&self.view.provider),
                    egui::Button::new("⟳").small().frame(false),
                );
                if refresh.on_hover_text("刷新模型").clicked() {
                    self.load_models(ctx, self.view.provider, true);
                }
                let shortcut = ctx.memory(|memory| memory.has_focus(editor_id))
                    && ctx.input(|input| {
                        input.modifiers.command && input.key_pressed(egui::Key::Enter)
                    });
                let ready = !busy
                    && (!self.view.draft.trim().is_empty() || !self.view.attachments.is_empty())
                    && (self.view.id.is_some() || self.folder.is_dir());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let send =
                        egui::Button::new(RichText::new("发送").color(PAPER).strong()).fill(ACCENT);
                    if ui.add_enabled(ready, send).clicked() || (shortcut && ready) {
                        self.submit(ctx);
                    }
                    if let Some(id) = &self.view.run
                        && ui.button("停止").clicked()
                        && let Ok(run) = self.service.run(id)
                    {
                        run.stop();
                    }
                });
            });
            if let Some(Err(error)) = self.models.get(&self.view.provider) {
                ui.label(RichText::new(error).color(WARN).size(12.));
            }
            if let Some(note) = &self.note {
                ui.label(RichText::new(note).color(WARN));
            }
        });
    }

    fn draft_images(&mut self, ui: &mut egui::Ui) {
        let busy = self.view.busy();
        let mut remove = None;
        if !self.view.attachments.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for image in &self.view.attachments {
                    if ui
                        .push_id(image.id(), |ui| {
                            ui.add_enabled(
                                !busy,
                                egui::Button::new(format!("{}  ×", image.name())).small(),
                            )
                            .on_hover_text(format!(
                                "{} × {} · 点击移除",
                                image.width(),
                                image.height()
                            ))
                            .clicked()
                        })
                        .inner
                    {
                        remove = Some(image.id().to_owned());
                    }
                }
            });
        }
        if let Some(id) = remove {
            self.view.attachments.retain(|image| image.id() != id);
        }
    }

    fn phone(&mut self, ctx: &egui::Context) {
        if !self.pair_dialog {
            return;
        }
        let mut opened = true;
        let remote = self.gateway.remote().status();
        egui::Window::new("手机连接")
            .open(&mut opened)
            .collapsible(false)
            .resizable(false)
            .default_width(440.)
            .show(ctx, |ui| {
                if remote.connected {
                    ui.label(RichText::new(&remote.detail).color(ACCENT));
                    if let Some(url) = &remote.url {
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(url);
                            if ui.small_button("复制网页地址").clicked() {
                                ctx.copy_text(url.clone());
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(self.gateway.pairing_code())
                                .monospace()
                                .size(30.)
                                .color(ACCENT)
                                .strong(),
                        );
                        ui.label(RichText::new("配对码").color(MUTED));
                    });
                    ui.label(
                        RichText::new("Safari → 分享 → 添加到主屏幕")
                            .color(MUTED)
                            .size(13.),
                    );
                } else {
                    ui.label(RichText::new(&remote.detail).color(MUTED));
                }
                egui::CollapsingHeader::new("连接中继服务器")
                    .default_open(!remote.connected)
                    .show(ui, |ui| {
                        ui.label("服务器地址");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.relay_url)
                                .desired_width(f32::INFINITY)
                                .hint_text("https://narolink.example.com"),
                        );
                        ui.label("连接密钥");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.relay_token)
                                .password(true)
                                .desired_width(f32::INFINITY),
                        );
                        if ui
                            .button(if cfg!(windows) {
                                "保存并连接"
                            } else {
                                "连接"
                            })
                            .clicked()
                        {
                            let token = std::mem::take(&mut self.relay_token);
                            self.relay_note = self
                                .gateway
                                .remote()
                                .connect_from_desktop(&self.relay_url, token)
                                .err();
                        }
                        if let Some(note) = &self.relay_note {
                            ui.label(RichText::new(note).color(WARN));
                        }
                    });
                ui.separator();
                if ui.button("打开本机网页").clicked() {
                    let url = format!(
                        "{}/#pair={}",
                        self.gateway.local_url(),
                        self.gateway.pairing_code()
                    );
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
            });
        self.pair_dialog = opened;
    }
}

fn centered(ui: &mut egui::Ui, width: f32, render: impl FnOnce(&mut egui::Ui)) {
    let width = width.min(ui.available_width() - 4.).max(1.);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - width) / 2.).max(0.));
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            render(ui);
        });
    });
}

fn composer_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PAPER)
        .inner_margin(Margin::symmetric(13, 9))
        .stroke(Stroke::new(1., LINE))
        .corner_radius(18)
        .shadow(egui::epaint::Shadow {
            offset: [0, 4],
            blur: 18,
            spread: 1,
            color: Color32::from_black_alpha(19),
        })
}
fn card(color: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(color)
        .inner_margin(Margin::symmetric(14, 10))
        .stroke(Stroke::new(1., LINE))
        .corner_radius(12)
}

fn sidebar_row(ui: &mut egui::Ui, title: &str, detail: &str, selected: bool) -> bool {
    let size = egui::vec2(ui.available_width(), 47.);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if selected || response.hovered() {
            ui.painter().rect_filled(rect, 8, RAIL_CARD);
        }
        let max = rect.width() - 17.;
        for (text, font, color, y) in [(title, 14., RAIL_TEXT, 5.), (detail, 11.5, RAIL_DIM, 25.)] {
            let galley = ui.fonts_mut(|fonts| {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    text.chars().take(120).collect(),
                    FontId::proportional(font),
                    color,
                );
                job.wrap.max_width = max;
                job.wrap.max_rows = 1;
                fonts.layout_job(job)
            });
            ui.painter()
                .galley(rect.left_top() + egui::vec2(8., y), galley, color);
        }
    }
    response
        .on_hover_text(format!("{title}\n{detail}"))
        .clicked()
}

fn render_message(
    ui: &mut egui::Ui,
    cache: &mut egui_commonmark::CommonMarkCache,
    message: &Message,
    provider: Provider,
) {
    let user = message.role == Role::User;
    let surface = if user {
        card(Color32::from_rgb(233, 236, 250))
    } else {
        egui::Frame::new().inner_margin(Margin::symmetric(8, 7))
    };
    surface.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if user { "你" } else { provider.label() })
                    .size(13.)
                    .strong()
                    .color(ACCENT),
            );
            if !user {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("复制").clicked() {
                        ui.ctx().copy_text(message.text.clone());
                    }
                });
            }
        });
        if user {
            ui.label(&message.text);
        } else {
            cache.link_hooks_clear();
            for event in Parser::new_ext(&message.text, Options::all()) {
                if let Event::Start(Tag::Link { dest_url, .. }) = event
                    && !dest_url.starts_with("https://")
                    && !dest_url.starts_with("http://")
                {
                    cache.add_link_hook(dest_url.as_ref());
                }
            }
            CommonMarkViewer::new()
                .explicit_image_uri_scheme(true)
                .show(ui, cache, &message.text);
        }
    });
    ui.add_space(6.);
}
