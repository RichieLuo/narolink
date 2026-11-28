use super::thumbnails::{PromptImage, RemoteImage, ThumbnailSource};
use super::{style::*, *};
use crate::{
    engine::{
        Question, QuestionAnswers, ReasoningChoice, Role,
        attachments::UserAttachment,
        usage::TokenCounts,
        work::{Agent, AgentStatus, FileChange, ReplyPart, Step, StepKind, StepStatus, Work},
    },
    service::{Delivery, Run},
};
use eframe::egui::{
    Align, Atom, AtomLayout, AtomLayoutResponse, Atoms, Button, Color32, FontId, Label, Layout,
    Margin, Rect, RichText, Stroke, StrokeKind, TextEdit, TextStyle, Ui, WidgetInfo, WidgetType,
    vec2,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const COLUMN: f32 = 744.;
const TITLE_BAR_HEIGHT: f32 = 38.;
const WINDOW_BUTTONS_INSET: i8 = if cfg!(target_os = "macos") { 88 } else { 20 };
const QR_DISPLAY_SIZE: f32 = 128.;
const COMPOSER_MIN_EDITOR_ROWS: usize = 2;
const COMPOSER_MAX_EDITOR_HEIGHT: f32 = 172.;
const ACTION_DIALOG_INSET: i8 = 22;

fn action_menu_row(ui: &mut Ui, label: &str) -> bool {
    let label = Language::in_ui(ui).text(label);
    ui.add(
        Button::new((label, Atom::grow()))
            .frame_when_inactive(false)
            .min_size(vec2(ui.available_width(), 30.)),
    )
    .clicked()
}

fn action_dialog_field(ui: &mut Ui, label: &str, edit: TextEdit<'_>) -> egui::Response {
    let palette = Palette::of(ui);
    let label = ui.label(RichText::new(label).size(13.).color(palette.muted));
    input::text_edit(
        ui,
        edit.desired_width(f32::INFINITY)
            .background_color(palette.canvas)
            .margin(Margin::symmetric(12, 10))
            .min_size(vec2(0., 42.)),
        false,
    )
    .labelled_by(label.id)
}

fn action_dialog_primary(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    let palette = Palette::of(ui);
    let (fill, hover, text) = if enabled {
        (palette.accent, palette.accent_hover, palette.on_accent)
    } else {
        (palette.soft, palette.soft, palette.muted)
    };
    solid(ui, fill, hover, |ui| {
        ui.add_enabled(
            enabled,
            Button::new(RichText::new(label).strong().color(text))
                .min_size(vec2(80., 36.))
                .corner_radius(10),
        )
    })
    .clicked()
}

enum ActionDialogChoice {
    Submit(engine::SessionAction),
    Command(String),
    Close,
}

fn action_dialog_header(ui: &mut Ui, dialog: &ActionDialog) -> bool {
    let language = Language::in_ui(ui);
    let palette = Palette::of(ui);
    let is_rename = matches!(&dialog.body, ActionBody::Rename { .. });
    let title_size = 21.;
    let subtitle_size = 12.;
    let spacing = 3.;
    let text_height = ui.fonts_mut(|fonts| {
        let title = fonts.row_height(&FontId::proportional(title_size));
        if is_rename {
            title
        } else {
            title + spacing + fonts.row_height(&FontId::proportional(subtitle_size))
        }
    });
    let icon = match &dialog.body {
        ActionBody::Rename { .. } => Icon::Pencil,
        ActionBody::Goal { .. } => Icon::Check,
        ActionBody::Review { .. } => Icon::Code,
        ActionBody::Rewind { .. } => Icon::Refresh,
        ActionBody::Context => Icon::Usage,
        ActionBody::Extensions { .. } => Icon::Terminal,
        ActionBody::Confirm { .. } => Icon::Folder,
    };
    egui::Frame::new()
        .inner_margin(Margin {
            left: ACTION_DIALOG_INSET,
            right: 16,
            top: 17,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 12.;
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), text_height.max(36.)),
                Layout::right_to_left(Align::Center),
                |ui| {
                    let close = Icon::Close
                        .button(ui, language.text("关闭"), palette.muted)
                        .clicked();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(36., 36.), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 11, palette.tint);
                        icon.paint(ui.painter(), rect.center(), palette.accent);
                        let title_width = ui.available_width();
                        ui.allocate_ui_with_layout(
                            vec2(title_width, text_height),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(title_width);
                                ui.spacing_mut().item_spacing.y = spacing;
                                ui.add(
                                    Label::new(
                                        RichText::new(language.text(dialog.body.title()))
                                            .size(title_size)
                                            .strong(),
                                    )
                                    .truncate(),
                                );
                                if !is_rename {
                                    ui.add(
                                        Label::new(
                                            RichText::new(&dialog.session.title)
                                                .size(subtitle_size)
                                                .color(palette.muted),
                                        )
                                        .truncate(),
                                    );
                                }
                            },
                        );
                    });
                    close
                },
            )
            .inner
        })
        .inner
}

fn action_dialog_footer(
    ui: &mut Ui,
    dialog: &ActionDialog,
    busy: bool,
) -> Option<ActionDialogChoice> {
    use engine::SessionAction as A;
    let language = Language::in_ui(ui);
    let palette = Palette::of(ui);
    egui::Frame::new()
        .fill(palette.canvas)
        .corner_radius(egui::CornerRadius {
            sw: 17,
            se: 17,
            ..Default::default()
        })
        .inner_margin(Margin::symmetric(ACTION_DIALOG_INSET, 13))
        .show(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut choice = None;
                let mut dismiss = "关闭";
                match &dialog.body {
                    ActionBody::Rename { name } => {
                        dismiss = "取消";
                        let valid = !name.trim().is_empty()
                            && name.len() <= 512
                            && !name.chars().any(char::is_control);
                        if action_dialog_primary(ui, language.text("保存"), !busy && valid) {
                            choice = Some(ActionDialogChoice::Submit(A::Rename {
                                name: name.trim().into(),
                            }));
                        }
                    }
                    ActionBody::Goal {
                        objective,
                        budget,
                        loaded: true,
                    } => {
                        dismiss = "取消";
                        let token_budget = budget.trim().parse::<u64>().ok();
                        let valid = (budget.trim().is_empty()
                            || token_budget.is_some_and(|n| n > 0))
                            && !objective.trim().is_empty();
                        if action_dialog_primary(ui, language.text("保存"), !busy && valid) {
                            choice = Some(ActionDialogChoice::Submit(A::GoalSet {
                                objective: objective.trim().into(),
                                token_budget,
                            }));
                        }
                    }
                    ActionBody::Review {
                        branch_mode,
                        branch,
                    } => {
                        dismiss = "取消";
                        let valid = !*branch_mode
                            || (!branch.trim().is_empty() && !branch.chars().any(char::is_control));
                        if action_dialog_primary(ui, language.text("填入输入框"), !busy && valid)
                        {
                            let command = if *branch_mode {
                                format!("/review {}", branch.trim())
                            } else {
                                "/review".into()
                            };
                            choice = Some(ActionDialogChoice::Command(command));
                        }
                    }
                    ActionBody::Confirm { label, action } if dialog.status.is_none() => {
                        dismiss = "取消";
                        if action_dialog_primary(ui, language.text(label), !busy) {
                            choice = Some(ActionDialogChoice::Submit(action.clone()));
                        }
                    }
                    _ => {}
                }
                let dismiss = if dialog.pending { "关闭" } else { dismiss };
                let dismiss_button = ui.add(
                    Button::new(language.text(dismiss))
                        .min_size(vec2(72., 36.))
                        .corner_radius(10),
                );
                if dismiss_button.clicked() {
                    return Some(ActionDialogChoice::Close);
                }
                if dialog.pending {
                    dismiss_button.on_hover_text(language.text("关闭不会取消已提交操作。"));
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Spinner::new().size(14.).color(palette.accent));
                        ui.label(RichText::new(language.text("正在处理…")).color(palette.muted));
                    });
                } else if matches!(&dialog.body, ActionBody::Goal { loaded: true, .. }) {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let clear = Button::new(language.text("清除目标"))
                            .frame_when_inactive(false)
                            .min_size(vec2(0., 36.));
                        if ui.add_enabled(!busy, clear).clicked() {
                            choice = Some(ActionDialogChoice::Submit(A::GoalClear));
                        }
                    });
                }
                choice
            })
            .inner
        })
        .inner
}

fn startup_settings(ui: &mut Ui, draft: &mut StartupDraft, provider: Provider, new: bool) {
    let language = Language::in_ui(ui);
    let palette = Palette::of(ui);
    ui.label(
        RichText::new(language.text("此设置仅用于下一次启动，不会改变正在运行的回复。"))
            .small()
            .color(palette.muted),
    );
    ui.label(language.text("附加目录"));
    ui.add(
        TextEdit::multiline(&mut draft.additional_directories)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text(language.text("每行一个绝对路径")),
    );
    ui.label(language.text("文件上下文"));
    ui.add(
        TextEdit::multiline(&mut draft.context_files)
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text(language.text("每行一个绝对路径")),
    );
    if provider != Provider::Codex {
        ui.label(language.text("Agent 名称"));
        ui.add(TextEdit::singleline(&mut draft.agent).desired_width(f32::INFINITY));
    }
    if new && provider == Provider::Claude {
        ui.label(language.text("独立 Worktree"));
        ui.add(TextEdit::singleline(&mut draft.worktree).desired_width(f32::INFINITY));
    }
}

fn render_context_details(ui: &mut Ui, report: Option<&engine::work::ContextUsage>) {
    let language = Language::in_ui(ui);
    let palette = Palette::of(ui);
    ui.label(
        RichText::new(language.text("最近一次引擎报告"))
            .small()
            .color(palette.muted),
    );
    for (label, value) in [
        ("已用", report.and_then(|report| report.used)),
        ("上限", report.and_then(|report| report.limit)),
        ("剩余", report.and_then(|report| report.remaining)),
    ] {
        let value = value
            .map(|n| n.to_string())
            .unwrap_or_else(|| language.text("引擎未报告").into());
        ui.label(format!("{} · {value}", language.text(label)));
    }
    ui.add_space(8.);
}

fn render_rewind_preview(ui: &mut Ui, value: &serde_json::Value) {
    let language = Language::in_ui(ui);
    let palette = Palette::of(ui);
    ui.separator();
    ui.label(
        RichText::new(language.text("文件列表与增删统计，不是完整差异。"))
            .small()
            .color(palette.muted),
    );
    if let Some(files) = value["filesChanged"].as_array() {
        for file in files {
            if let Some(path) = file.as_str().or_else(|| file["path"].as_str()) {
                ui.add(Label::new(path).wrap().selectable(true));
            }
        }
        if files.is_empty() {
            ui.label(language.text("没有需要回退的文件"));
        }
    } else {
        ui.label(language.text("引擎未返回文件列表"));
    }
    let number = |key: &str| {
        value[key]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| language.text("未知").into())
    };
    ui.label(format!(
        "+{} / −{}",
        number("insertions"),
        number("deletions")
    ));
}

fn render_mcp_servers(ui: &mut Ui, value: &serde_json::Value) {
    let language = Language::in_ui(ui);
    let palette = Palette::of(ui);
    let servers = value["data"]
        .as_array()
        .or_else(|| value["mcpServers"].as_array())
        .or_else(|| value["servers"].as_array())
        .or_else(|| value.as_array());
    let Some(servers) = servers else {
        ui.label(
            RichText::new(language.text("当前引擎未报告可读条目"))
                .small()
                .color(palette.muted),
        );
        return;
    };
    if servers.is_empty() {
        ui.label(language.text("没有已配置的 MCP 服务器"));
    }
    for (index, server) in servers.iter().enumerate() {
        ui.push_id(index, |ui| {
            let name = server["name"]
                .as_str()
                .unwrap_or(language.text("未命名服务器"));
            ui.strong(name);
            for (label, key) in [("连接状态", "status"), ("授权状态", "authStatus")] {
                if let Some(status) = server[key].as_str().or_else(|| {
                    (key == "status")
                        .then(|| server["runtimeStatus"].as_str())
                        .flatten()
                }) {
                    let status = match status {
                        "connected" => "已连接",
                        "failed" => "连接失败",
                        "pending" => "连接中",
                        "disabled" => "已停用",
                        "needs-auth" | "notLoggedIn" => "需要授权",
                        "oAuth" => "已授权",
                        "bearerToken" => "已配置令牌",
                        "unsupported" => "不支持授权",
                        other => other,
                    };
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            language.text(label),
                            language.text(status)
                        ))
                        .small()
                        .color(palette.muted),
                    );
                }
            }
            if let Some(error) = server["error"].as_str() {
                ui.label(RichText::new(error).color(palette.warn));
            }
            let tools: Vec<&str> = if let Some(tools) = server["tools"].as_object() {
                tools.keys().map(String::as_str).collect()
            } else if let Some(tools) = server["tools"].as_array() {
                tools
                    .iter()
                    .filter_map(|tool| tool.as_str().or_else(|| tool["name"].as_str()))
                    .collect()
            } else {
                Vec::new()
            };
            if !tools.is_empty() {
                ui.collapsing(language.text("工具"), |ui| {
                    for name in tools {
                        ui.add(Label::new(name).wrap());
                    }
                });
            }
            ui.add_space(6.);
        });
    }
}

fn render_extensions(ui: &mut Ui, value: &serde_json::Value, full: bool) {
    let language = Language::in_ui(ui);
    let palette = Palette::of(ui);
    if !full {
        render_mcp_servers(ui, value);
        return;
    }
    ui.strong(language.text("MCP 状态"));
    render_mcp_servers(ui, &value["mcp"]);
    for (label, key, error_key) in [
        ("Hooks", "hooks", "hooks_error"),
        ("插件", "plugins", "plugins_error"),
        ("应用", "apps", "apps_error"),
    ] {
        ui.separator();
        ui.strong(language.text(label));
        if let Some(error) = value[error_key].as_str() {
            ui.label(RichText::new(error).color(palette.warn));
        } else {
            let result = &value[key];
            let rows = result
                .as_array()
                .or_else(|| result["data"].as_array())
                .or_else(|| result[key].as_array())
                .or_else(|| result["installed"].as_array());
            if let Some(rows) = rows {
                for row in rows {
                    if let Some(name) = row["name"].as_str().or_else(|| row["displayName"].as_str())
                    {
                        ui.add(Label::new(name).wrap());
                    }
                }
                if rows.is_empty() {
                    ui.label(
                        RichText::new(language.text("无条目"))
                            .small()
                            .color(palette.muted),
                    );
                } else if !rows
                    .iter()
                    .any(|row| row["name"].is_string() || row["displayName"].is_string())
                {
                    ui.label(
                        RichText::new(language.text("已读取状态；详细配置请在原生 CLI 中查看。"))
                            .small()
                            .color(palette.muted),
                    );
                }
            } else {
                ui.label(
                    RichText::new(language.text(if result.is_null() {
                        "引擎未报告"
                    } else {
                        "已读取状态；详细配置请在原生 CLI 中查看。"
                    }))
                    .small()
                    .color(palette.muted),
                );
            }
        }
    }
}

fn window_header(
    ctx: &egui::Context,
    id: &'static str,
    left_inset: i8,
    layout: Layout,
    contents: impl FnOnce(&mut Ui),
) {
    let palette = Palette::from_context(ctx);
    egui::TopBottomPanel::top(id)
        .exact_height(TITLE_BAR_HEIGHT)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(palette.paper).inner_margin(Margin {
            left: left_inset,
            right: 14,
            ..Margin::ZERO
        }))
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                (rect.left() - f32::from(left_inset))..=(rect.right() + 14.),
                rect.bottom() - 0.5,
                Stroke::new(1., palette.line),
            );
            ui.with_layout(layout, contents);
        });
}

fn window_drag_area(ui: &Ui, rect: Rect) {
    if cfg!(target_os = "macos") && rect.is_positive() {
        let area = ui.interact(rect, ui.id().with("window drag area"), egui::Sense::drag());
        if area.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }
}

pub(super) struct PairingQr {
    link: String,
    texture: Option<egui::TextureHandle>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) enum SettingsTab {
    #[default]
    General,
    Session,
    Cli,
    Updates,
}

impl SettingsTab {
    const ALL: [Self; 4] = [Self::General, Self::Session, Self::Cli, Self::Updates];

    fn label(self) -> &'static str {
        match self {
            Self::General => "通用",
            Self::Session => "会话",
            Self::Cli => "CLI",
            Self::Updates => "软件更新",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) enum DeviceTab {
    #[default]
    Connect,
    PairThisComputer,
}

impl DeviceTab {
    fn label(self) -> &'static str {
        match self {
            Self::Connect => "连接其他设备",
            Self::PairThisComputer => "本机配对",
        }
    }
}

impl PairingQr {
    fn new(ctx: &egui::Context, link: String) -> Self {
        let texture = qrcode::QrCode::new(&link).ok().map(|code| {
            const QUIET_ZONE: usize = 4;
            let width = code.width();
            let size = width + 2 * QUIET_ZONE;
            let mut image = egui::ColorImage::filled([size, size], Color32::WHITE);
            for (index, color) in code.into_colors().into_iter().enumerate() {
                if color == qrcode::Color::Dark {
                    let x = index % width + QUIET_ZONE;
                    let y = index / width + QUIET_ZONE;
                    image[(x, y)] = Color32::BLACK;
                }
            }
            ctx.load_texture("device pairing QR", image, egui::TextureOptions::NEAREST)
        });
        Self { link, texture }
    }

    fn paint(texture: &egui::TextureHandle, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let (tile, _) =
            ui.allocate_exact_size(vec2(QR_DISPLAY_SIZE, QR_DISPLAY_SIZE), egui::Sense::hover());
        ui.painter().rect(
            tile,
            12.,
            Color32::WHITE,
            Stroke::new(1., palette.tint_line),
            StrokeKind::Inside,
        );
        // Align the image to whole physical pixels so each module has crisp, even edges.
        let pixels_per_point = ui.pixels_per_point();
        let modules = texture.size()[0] as f32;
        let module_pixels = (((QR_DISPLAY_SIZE - 12.) * pixels_per_point) / modules)
            .floor()
            .max(1.);
        let size = modules * module_pixels / pixels_per_point;
        let origin = tile.center() - vec2(size, size) / 2.;
        let origin = (origin * pixels_per_point).round() / pixels_per_point;
        ui.painter().image(
            texture.id(),
            Rect::from_min_size(origin, vec2(size, size)),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
            Color32::WHITE,
        );
    }
}

fn pairing_link(address: &str, code: &str) -> String {
    format!("{}/#pair={code}", address.trim_end_matches('/'))
}

struct SessionGroup {
    key: egui::Id,
    project_id: Option<String>,
    path: PathBuf,
    name: String,
    rows: Vec<Session>,
}

pub(super) struct QuestionForm {
    pub(super) id: String,
    open: bool,
    drafts: Vec<QuestionDraft>,
    pub(super) error: Option<String>,
}

#[derive(Default)]
struct QuestionDraft {
    selected: BTreeSet<usize>,
    use_custom: bool,
    custom: String,
}

impl QuestionDraft {
    fn show(&mut self, ui: &mut Ui, question: &Question) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        if !question.header.is_empty() && question.header != question.text {
            ui.label(
                RichText::new(&question.header)
                    .color(palette.muted)
                    .size(12.),
            );
        }
        ui.label(RichText::new(&question.text).strong());
        for (index, option) in question.options.iter().enumerate() {
            let mut selected = self.selected.contains(&index);
            if question.multiple {
                if ui.checkbox(&mut selected, &option.label).changed() {
                    if selected {
                        self.selected.insert(index);
                    } else {
                        self.selected.remove(&index);
                    }
                }
            } else if ui.radio(selected, &option.label).clicked() {
                self.selected.clear();
                self.selected.insert(index);
                self.use_custom = false;
            }
            if !option.description.is_empty() {
                ui.label(
                    RichText::new(&option.description)
                        .color(palette.muted)
                        .size(12.5),
                );
            }
        }
        if question.allow_custom && !question.options.is_empty() {
            if question.multiple {
                ui.checkbox(&mut self.use_custom, language.text("自定义答案"));
            } else if ui
                .radio(self.use_custom, language.text("自定义答案"))
                .clicked()
            {
                self.use_custom = true;
                self.selected.clear();
            }
        }
        if self.use_custom {
            input::text_edit(
                ui,
                TextEdit::singleline(&mut self.custom)
                    .desired_width(f32::INFINITY)
                    .hint_text(language.text("输入答案")),
                question.secret,
            );
        }
    }

    fn answers(&self, question: &Question) -> Vec<String> {
        let mut answers: Vec<_> = self
            .selected
            .iter()
            .map(|&index| question.options[index].label.clone())
            .collect();
        if self.use_custom {
            let value = self.custom.trim();
            if !answers.iter().any(|answer| answer == value) {
                answers.push(value.to_owned());
            }
        }
        answers
    }
}

impl Desktop {
    pub(super) fn paint(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        if self.settings_page.is_some() {
            self.settings(ctx);
            return;
        }
        // Keep both the chat and right panel readable on smaller windows. The session
        // list returns when the panel closes or the window is widened.
        self.sync_git_cwd(ctx);
        if self.files.is_open() {
            self.git.close();
            self.tree.close();
        }
        let show_sidebar = !(self.files.is_open() || self.git.is_open() || self.tree.is_open())
            || ctx.content_rect().width() >= 1080.;
        if show_sidebar {
            self.sidebar(ctx);
        }
        self.conversation_header(ctx, show_sidebar);
        if self.files.show(ctx) {
            self.files.close();
            self.tree.open();
        }
        let status = self.git.status.as_ref().filter(|status| status.is_repo);
        if let Some(command) = self
            .tree
            .show(ctx, status, self.git.serial, self.git.loading)
        {
            self.handle_tree_command(ctx, command);
        }
        if let Some(command) = self.git.show(ctx) {
            self.handle_git_command(ctx, command);
        }

        let chat_bounds = ctx.available_rect();
        let editor_max_height = (chat_bounds.height() * 0.3).min(COMPOSER_MAX_EDITOR_HEIGHT);
        let composer = egui::TopBottomPanel::bottom("composer")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.paper)
                    .inner_margin(Margin::symmetric(20, 16)),
            )
            .show(ctx, |ui| {
                centered(ui, COLUMN, |ui| self.composer(ui, ctx, editor_max_height));
            });
        if (composer.response.rect.bottom() - chat_bounds.bottom()).abs() > 0.5 {
            ctx.request_repaint();
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.paper)
                    .inner_margin(Margin::symmetric(20, 0)),
            )
            .show(ctx, |ui| {
                let empty = self.view.is_new();
                let menu_press = chat_menu::begin(ui, ui.max_rect());
                egui::ScrollArea::vertical()
                    .id_salt(("chat", self.epoch))
                    .stick_to_bottom(!empty)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if empty {
                            ui.add_space((ui.available_height() * 0.19).clamp(24., 104.));
                            centered(ui, 450., |ui| self.new_session_form(ui, ctx));
                        } else {
                            ui.add_space(24.);
                            centered(ui, COLUMN, |ui| self.conversation(ui));
                        }
                        ui.add_space(20.);
                    });
                if let Some(path) = chat_menu::show(ui, menu_press) {
                    let cwd = self
                        .view
                        .session
                        .as_ref()
                        .map_or_else(|| self.folder.clone(), |session| session.cwd.clone());
                    self.files.open(ctx, path, &cwd);
                }
            });
        self.devices(ctx);
        if self.quota_dialog {
            self.paint_quota_dialog(ctx);
        } else {
            self.questions(ctx);
        }
        self.paint_action_dialog(ctx);
        if let Some(command) = self.git.show_diff_modal(ctx) {
            self.handle_git_command(ctx, command);
        }
        self.thumbnails.show_preview(ctx);
        #[cfg(not(target_os = "macos"))]
        if let Some((editor_id, dialog)) = &mut self.attachment_picker {
            let editor_id = *editor_id;
            dialog.update(ctx);
            if let Some(paths) = dialog.take_picked_multiple() {
                self.attachment_picker = None;
                self.accept_attachments(ctx, editor_id, paths);
            } else if matches!(dialog.state(), egui_file_dialog::DialogState::Cancelled) {
                self.attachment_picker = None;
            }
        }
    }

    fn conversation_header(&mut self, ctx: &egui::Context, show_sidebar: bool) {
        let palette = Palette::from_context(ctx);
        let language = self.language;
        let inset = if show_sidebar {
            20
        } else {
            WINDOW_BUTTONS_INSET
        };
        window_header(
            ctx,
            "header",
            inset,
            Layout::right_to_left(Align::Center),
            |ui| {
                let device_label = match &self.remote {
                    Some(remote) => {
                        language.format("当前设备 · {name}", &[("name", remote.name())])
                    }
                    None => language.text("设备连接").to_owned(),
                };
                let devices = Icon::Devices
                    .label(RichText::new(engine::cut(&device_label, 32)).size(12.))
                    .atom_ui(ui);
                Icon::Devices.paint_in(
                    ui,
                    &devices,
                    if self.remote.is_some() {
                        palette.accent
                    } else {
                        palette.muted
                    },
                );
                if devices.response.on_hover_text(device_label).clicked() {
                    self.pair_dialog = true;
                }
                if self.current_git_cwd().is_some() {
                    let git_label = match self.git.status.as_ref().filter(|st| st.is_repo) {
                        Some(st) => {
                            let name = st
                                .branch
                                .as_deref()
                                .or(st.head_short.as_deref())
                                .unwrap_or("Git");
                            let changes = st.total_changes();
                            if changes > 0 {
                                format!("{} · {changes}", engine::cut(name, 16))
                            } else {
                                engine::cut(name, 18).to_owned()
                            }
                        }
                        None => "Git".to_owned(),
                    };
                    let git_btn = Icon::GitBranch
                        .label(RichText::new(&git_label).size(12.))
                        .selected(self.git.is_open())
                        .atom_ui(ui);
                    Icon::GitBranch.paint_in(
                        ui,
                        &git_btn,
                        if self.git.is_open() {
                            palette.accent
                        } else {
                            palette.muted
                        },
                    );
                    if git_btn
                        .response
                        .on_hover_text(language.text("Git 管理"))
                        .clicked()
                    {
                        self.toggle_git_panel(ctx);
                    }
                    // File previews read from this machine, so remote workspaces have no tree.
                    if self.remote.is_none() {
                        let open = self.tree.is_open();
                        let tree_btn = Icon::Folder
                            .label(RichText::new(language.text("文件")).size(12.))
                            .selected(open)
                            .atom_ui(ui);
                        Icon::Folder.paint_in(
                            ui,
                            &tree_btn,
                            if open { palette.accent } else { palette.muted },
                        );
                        if tree_btn
                            .response
                            .on_hover_text(language.text("文件目录"))
                            .clicked()
                        {
                            self.toggle_tree_panel(ctx);
                        }
                    }
                }
                if self.view.session.is_some() {
                    let trigger = Icon::More.button(ui, language.text("会话操作"), palette.muted);
                    egui::Popup::menu(&trigger)
                        .frame(popup(palette, Margin::same(8)))
                        .show(|ui| {
                            ui.set_width(204.);
                            self.session_action_menu(ui, ctx);
                        });
                }
                if self.view.title.is_some() {
                    ui.label(
                        RichText::new(self.view.provider.label())
                            .color(palette.muted)
                            .size(11.5),
                    );
                    let (dot, _) = ui.allocate_exact_size(vec2(4., 4.), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 2., palette.accent);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    self.conversation_title(ui);
                    window_drag_area(ui, ui.available_rect_before_wrap());
                });
            },
        );
    }

    fn stage_command(&mut self, command: String) {
        self.view.show_draft(ComposerDraft {
            text: command,
            images: Vec::new(),
            local_files: Vec::new(),
            startup: self.view.startup.clone(),
            withdrawn: false,
        });
        self.restore_composer_focus = true;
    }

    fn session_action_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        use engine::SessionAction as A;
        if action_menu_row(ui, "上下文用量") {
            self.open_action(ActionBody::Context);
            ui.close();
            return;
        }
        if self.view.provider == Provider::Antigravity {
            return;
        }
        ui.separator();
        let language = self.language;
        let ready = !self.view.busy() && !self.native_pending();
        if !ready {
            ui.label(
                RichText::new(language.text("运行或操作结束后可用"))
                    .small()
                    .color(Palette::of(ui).muted),
            );
        }
        let mut body = None;
        let mut load = None;
        ui.add_enabled_ui(ready, |ui| {
            if action_menu_row(ui, "重命名") {
                body = Some(ActionBody::Rename {
                    name: self.view.title.clone().unwrap_or_default(),
                });
            }
            if action_menu_row(ui, "创建分支") {
                body = Some(ActionBody::Confirm {
                    label: "创建分支",
                    action: A::Fork,
                });
            }
            match self.view.provider {
                Provider::Codex => {
                    if action_menu_row(ui, "目标") {
                        body = Some(ActionBody::Goal {
                            objective: String::new(),
                            budget: String::new(),
                            loaded: false,
                        });
                        load = Some(A::GoalGet);
                    }
                    if action_menu_row(ui, "Review") {
                        body = Some(ActionBody::Review {
                            branch_mode: false,
                            branch: String::new(),
                        });
                    }
                    if action_menu_row(ui, "归档") {
                        body = Some(ActionBody::Confirm {
                            label: "归档",
                            action: A::Archive,
                        });
                    }
                    // Session metadata does not identify archived rows, so restoration
                    // remains an explicit operation instead of guessing archive state.
                    if action_menu_row(ui, "恢复归档") {
                        body = Some(ActionBody::Confirm {
                            label: "恢复归档",
                            action: A::Unarchive,
                        });
                    }
                }
                Provider::Claude => {
                    if action_menu_row(ui, "文件回退") {
                        body = Some(ActionBody::Rewind {
                            selected: None,
                            preview: None,
                        });
                    }
                }
                Provider::Antigravity => {}
            }
            ui.separator();
            ui.collapsing(language.text("高级操作"), |ui| {
                if action_menu_row(ui, "MCP 状态") {
                    body = Some(ActionBody::Extensions {
                        result: None,
                        full: false,
                    });
                    load = Some(A::McpStatus);
                }
                if self.view.provider == Provider::Codex && action_menu_row(ui, "扩展状态") {
                    body = Some(ActionBody::Extensions {
                        result: None,
                        full: true,
                    });
                    load = Some(A::Extensions);
                }
            });
        });
        if let Some(body) = body {
            self.open_action(body);
            if let Some(action) = load {
                self.request_native_action(ctx, action);
            }
            ui.close();
        }
    }

    fn paint_action_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.action_dialog.take() else {
            return;
        };
        if dialog.epoch != self.epoch
            || self.view.id.as_deref() != Some(&dialog.session.id)
            || self.view.provider != dialog.session.provider
        {
            self.restore_composer_focus = true;
            return;
        }
        let palette = Palette::from_context(ctx);
        let busy = self.view.busy() || self.native_pending();
        let screen = ctx.content_rect();
        let width: f32 = if matches!(&dialog.body, ActionBody::Rename { .. }) {
            416.
        } else {
            464.
        };
        let id = egui::Id::new(("session action", dialog.id));
        let modal = egui::Modal::new(id)
            .frame(popup(palette, Margin::same(0)).corner_radius(18))
            .backdrop_color(palette.backdrop)
            .show(ctx, |ui| {
                ui.set_width(width.min((screen.width() - 32.).max(1.)));
                ui.spacing_mut().item_spacing.y = 0.;
                let close = action_dialog_header(ui, &dialog);
                let choice = egui::ScrollArea::vertical()
                    .id_salt("action body")
                    .auto_shrink([false, true])
                    .max_height((screen.height() - 176.).max(48.))
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin {
                                left: ACTION_DIALOG_INSET,
                                right: ACTION_DIALOG_INSET,
                                top: 7,
                                bottom: 21,
                            })
                            .show(ui, |ui| self.action_dialog_body(ui, &mut dialog, busy))
                            .inner
                    })
                    .inner;
                let footer = action_dialog_footer(ui, &dialog, busy);
                if close {
                    Some(ActionDialogChoice::Close)
                } else {
                    footer.or(choice)
                }
            });

        if modal.should_close() || matches!(&modal.inner, Some(ActionDialogChoice::Close)) {
            self.restore_composer_focus = true;
        } else if let Some(ActionDialogChoice::Command(command)) = modal.inner {
            self.stage_command(command);
        } else {
            self.action_dialog = Some(dialog);
            if let Some(ActionDialogChoice::Submit(action)) = modal.inner {
                self.request_native_action(ctx, action);
            }
        }
    }

    fn action_dialog_body(
        &self,
        ui: &mut Ui,
        dialog: &mut ActionDialog,
        busy: bool,
    ) -> Option<ActionDialogChoice> {
        use engine::SessionAction as A;
        let language = self.language;
        let palette = Palette::of(ui);
        let is_context = matches!(&dialog.body, ActionBody::Context);
        let mut choice = None;
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.;
        ui.add_enabled_ui(!busy || is_context, |ui| match &mut dialog.body {
            ActionBody::Rename { name } => {
                let edit =
                    action_dialog_field(ui, language.text("会话名称"), TextEdit::singleline(name));
                if !ui.memory(|memory| memory.areas().visible_last_frame(&ui.layer_id())) && !busy {
                    edit.request_focus();
                }
            }
            ActionBody::Goal {
                objective,
                budget,
                loaded,
            } => {
                if !*loaded {
                    if ui.button(language.text("重新加载目标")).clicked() {
                        choice = Some(ActionDialogChoice::Submit(A::GoalGet));
                    }
                } else {
                    action_dialog_field(
                        ui,
                        language.text("目标"),
                        TextEdit::multiline(objective).desired_rows(3),
                    );
                    ui.add_space(3.);
                    action_dialog_field(
                        ui,
                        language.text("Token 预算"),
                        TextEdit::singleline(budget).hint_text(language.text("可选，留空不设预算")),
                    );
                    let valid_budget = budget.trim().is_empty()
                        || budget.trim().parse::<u64>().is_ok_and(|n| n > 0);
                    if !valid_budget {
                        ui.colored_label(palette.warn, language.text("请输入大于零的 Token 预算"));
                    }
                }
            }
            ActionBody::Review {
                branch_mode,
                branch,
            } => {
                ui.radio_value(branch_mode, false, language.text("未提交改动"));
                ui.radio_value(branch_mode, true, language.text("指定分支"));
                if *branch_mode {
                    action_dialog_field(
                        ui,
                        language.text("分支名称"),
                        TextEdit::singleline(branch),
                    );
                }
                ui.label(
                    RichText::new(language.text("返回输入框确认发送"))
                        .small()
                        .color(palette.muted),
                );
            }
            ActionBody::Rewind { selected, preview } => {
                ui.label(language.text("仅回退文件，不回退对话"));
                ui.add_space(6.);
                let mut choices = 0;
                for (index, message) in self
                    .view
                    .transcript
                    .messages
                    .iter()
                    .filter(|message| message.role == Role::User)
                    .enumerate()
                {
                    let Some(id) = message
                        .turn_id
                        .as_ref()
                        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    else {
                        continue;
                    };
                    choices += 1;
                    let summary = format!(
                        "{}. {}",
                        index + 1,
                        engine::short(
                            &message
                                .text
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" "),
                            90
                        )
                    );
                    if ui
                        .add(
                            Button::new(summary)
                                .selected(selected.as_ref() == Some(id))
                                .wrap(),
                        )
                        .clicked()
                    {
                        if selected.as_ref() != Some(id) {
                            *selected = Some(id.clone());
                            *preview = None;
                        }
                    }
                }
                if choices == 0 {
                    ui.label(language.text("暂无可回退的用户消息"));
                }
                if let Some(id) = selected {
                    if ui.button(language.text("预览文件回退")).clicked() {
                        *preview = None;
                        choice = Some(ActionDialogChoice::Submit(A::CheckpointPreview {
                            message_id: id.clone(),
                        }));
                    }
                    if let Some((target, value)) =
                        preview.as_ref().filter(|(target, _)| target == id)
                    {
                        render_rewind_preview(ui, value);
                        if ui.button(language.text("确认文件回退")).clicked() {
                            choice = Some(ActionDialogChoice::Submit(A::CheckpointRewind {
                                message_id: target.clone(),
                            }));
                        }
                    }
                }
            }
            ActionBody::Context => {
                render_context_details(ui, self.view.latest_context());
                if !busy && dialog.session.provider != Provider::Antigravity {
                    ui.horizontal_wrapped(|ui| {
                        if dialog.session.provider == Provider::Claude
                            && ui.button(language.text("刷新上下文")).clicked()
                        {
                            choice = Some(ActionDialogChoice::Submit(A::Context));
                        }
                        if ui.button(language.text("压缩上下文")).clicked() {
                            choice = Some(ActionDialogChoice::Command("/compact".into()));
                        }
                    });
                    ui.label(
                        RichText::new(language.text("返回输入框确认发送"))
                            .small()
                            .color(palette.muted),
                    );
                }
            }
            ActionBody::Extensions { result, full } => {
                if let Some(value) = result {
                    render_extensions(ui, value, *full);
                }
                if ui.button(language.text("刷新")).clicked() {
                    choice = Some(ActionDialogChoice::Submit(if *full {
                        A::Extensions
                    } else {
                        A::McpStatus
                    }));
                }
                if dialog.session.provider == Provider::Codex
                    && ui.button(language.text("重新加载 MCP")).clicked()
                {
                    choice = Some(ActionDialogChoice::Submit(A::McpReconnect));
                }
            }
            ActionBody::Confirm {
                action: confirmed, ..
            } => {
                ui.label(language.text(match confirmed {
                    A::Archive => "归档后会话仍可通过原生搜索恢复。",
                    A::Fork => "创建独立的原生会话分支，原会话保持不变。",
                    _ => "恢复后会话将重新出现在历史列表。",
                }));
            }
        });
        if busy
            && !dialog.pending
            && (!is_context || dialog.session.provider != Provider::Antigravity)
        {
            ui.label(
                RichText::new(language.text("运行或操作结束后可用"))
                    .small()
                    .color(palette.muted),
            );
        }
        if let Some(status) = dialog.status {
            ui.label(RichText::new(language.text(status)).color(palette.muted));
        }
        if let Some(error) = &dialog.error {
            egui::Frame::new()
                .fill(palette.danger_surface)
                .corner_radius(9)
                .inner_margin(Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(language.text(error))
                            .size(13.)
                            .color(palette.danger),
                    );
                });
        }
        choice
    }

    fn conversation_title(&self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let title = self
            .view
            .title
            .as_deref()
            .unwrap_or(self.language.text("工作台"));
        let text_width = (ui.available_width() - 24.).max(0.);
        let project_width = if self.view.session.is_some() {
            (text_width * 0.34).min(220.)
        } else {
            0.
        };
        ui.scope(|ui| {
            ui.set_max_width(text_width - project_width);
            ui.add(Label::new(RichText::new(title).size(13.)).truncate())
                .on_hover_text(title);
        });
        if let Some(session) = &self.view.session {
            let name = session
                .project_id
                .as_deref()
                .and_then(|id| self.codex_project(id))
                .map(|project| project.name.as_str())
                .or_else(|| session.cwd.file_name().and_then(|name| name.to_str()));
            let path = session.cwd.to_string_lossy();
            ui.scope(|ui| {
                ui.set_max_width(project_width);
                ui.add(
                    Label::new(
                        RichText::new(name.unwrap_or(&path))
                            .color(palette.muted)
                            .size(11.5),
                    )
                    .truncate(),
                )
                .on_hover_text(path.as_ref());
            });
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        let language = self.language;
        let min_width = 240.;
        let max_width = (ctx.available_rect().width() - 480.).clamp(min_width, 420.);
        let style = ctx.style();
        // egui draws the resize edge with the panel's widget foreground strokes.
        ctx.style_mut(|style| {
            let widgets = &mut style.visuals.widgets;
            widgets.hovered.fg_stroke.color = palette.line;
            widgets.active.fg_stroke.color = palette.tint_line;
        });
        egui::SidePanel::left("sidebar")
            .default_width(258.)
            .width_range(min_width..=max_width)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(Margin::symmetric(13, 13)),
            )
            .show(ctx, |ui| {
                ctx.set_style(style.clone());
                ui.set_style(style);
                #[cfg(target_os = "macos")]
                {
                    let top = ui.max_rect().top() - 13.;
                    window_drag_area(
                        ui,
                        Rect::from_min_max(
                            egui::pos2(f32::from(WINDOW_BUTTONS_INSET), top),
                            egui::pos2(ui.max_rect().right(), top + TITLE_BAR_HEIGHT),
                        ),
                    );
                    ui.add_space(TITLE_BAR_HEIGHT);
                }
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new((self.brand.id(), vec2(34., 34.))).corner_radius(9));
                    ui.label(RichText::new("Naro").size(21.));
                });
                ui.add_space(12.);
                let width = ui.available_width();
                if solid(ui, palette.accent_soft, palette.tint, |ui| {
                    let button = Button::new((
                        Icon::Plus.atom(),
                        RichText::new(language.text("新建会话"))
                            .size(14.)
                            .color(palette.ink),
                        Atom::grow(),
                    ))
                    .min_size(vec2(width, 36.))
                    .atom_ui(ui);
                    Icon::Plus.paint_in(ui, &button, palette.ink);
                    button.response
                })
                .clicked()
                {
                    self.new_view(ctx, None, None);
                }

                ui.add_space(8.);
                let search_id = egui::Id::new("sidebar search");
                let search_focused = ui.memory(|mem| mem.has_focus(search_id));
                let search_focus_t =
                    ctx.animate_bool_with_time(search_id.with("focus"), search_focused, 0.14);
                egui::Frame::new()
                    .fill(palette.paper)
                    .stroke(Stroke::new(
                        1.,
                        lerp_color(palette.line, palette.accent, search_focus_t),
                    ))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(9, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
                            Icon::Search.paint(ui.painter(), rect.center(), palette.muted);
                            input::text_edit_with_menu(
                                ui,
                                search_id,
                                TextEdit::singleline(&mut self.search)
                                    .font(FontId::proportional(12.5))
                                    .hint_text(language.text("搜索会话或目录"))
                                    .frame(false)
                                    .desired_width(ui.available_width()),
                                false,
                                || false,
                            );
                        });
                    });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.;
                    ui.spacing_mut().button_padding.x = 6.;
                    for (provider, label) in [
                        (None, language.text("全部")),
                        (Some(Provider::Codex), "Codex"),
                        (Some(Provider::Claude), "Claude"),
                        (Some(Provider::Antigravity), "Antigravity"),
                    ] {
                        let active = self.filters == provider;
                        if ui
                            .add(
                                Button::new(RichText::new(label).size(11.5).color(if active {
                                    palette.ink
                                } else {
                                    palette.muted
                                }))
                                .min_size(vec2(0., 25.))
                                .small()
                                .selected(active)
                                .frame_when_inactive(active),
                            )
                            .clicked()
                        {
                            self.filters = provider;
                        }
                    }
                });
                self.sidebar_native_search(ui, ctx);
                ui.add_space(9.);
                ui.horizontal(|ui| {
                    ui.add_space(6.);
                    ui.label(
                        RichText::new(language.text("项目"))
                            .size(12.)
                            .color(palette.muted),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let spinning = !self.inventory_pending.is_empty();
                        if Icon::refresh_button(
                            ui,
                            language.text("刷新历史"),
                            palette.muted,
                            spinning,
                        )
                        .clicked()
                        {
                            self.refresh_inventory(ctx, true);
                        }
                    });
                });
                egui::TopBottomPanel::bottom("sidebar footer")
                    .show_separator_line(false)
                    .frame(
                        egui::Frame::new()
                            .fill(palette.sidebar)
                            .outer_margin(Margin {
                                top: 4,
                                ..Margin::ZERO
                            })
                            .inner_margin(Margin {
                                top: 8,
                                ..Margin::ZERO
                            }),
                    )
                    .show_inside(ui, |ui| {
                        let rect = ui.max_rect();
                        ui.painter().hline(
                            rect.left()..=rect.right(),
                            rect.top() - 8. + 0.5,
                            Stroke::new(1., palette.line),
                        );
                        let gap = 6.;
                        let button_width = ((ui.available_width() - gap) * 0.5).max(0.);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            let usage = Button::new((
                                Icon::Usage.atom(),
                                RichText::new(language.text("用量"))
                                    .size(12.5)
                                    .color(palette.ink),
                            ))
                            .min_size(vec2(button_width, 32.))
                            .frame_when_inactive(false)
                            .atom_ui(ui);
                            Icon::Usage.paint_in(
                                ui,
                                &usage,
                                if usage.response.hovered() {
                                    palette.accent
                                } else {
                                    palette.muted
                                },
                            );
                            if usage.response.clicked() {
                                self.quota_dialog = true;
                                self.showing_account_quota = false;
                                self.quota_provider = self.view.provider;
                                self.load_token_usage(ctx);
                                ctx.memory_mut(|memory| memory.stop_text_input());
                            }
                            let settings = Button::new((
                                Icon::Sliders.atom(),
                                RichText::new(language.text("设置"))
                                    .size(12.5)
                                    .color(palette.ink),
                            ))
                            .min_size(vec2(button_width, 32.))
                            .frame_when_inactive(false)
                            .atom_ui(ui);
                            Icon::Sliders.paint_in(
                                ui,
                                &settings,
                                if settings.response.hovered() {
                                    palette.accent
                                } else {
                                    palette.muted
                                },
                            );
                            if self.updates.has_update() {
                                let rect = settings.response.rect;
                                ui.painter().circle_filled(
                                    rect.right_center() - vec2(6., 0.),
                                    2.5,
                                    palette.accent,
                                );
                                settings
                                    .response
                                    .clone()
                                    .on_hover_text(language.text("有可用更新"));
                            }
                            if settings.response.clicked() {
                                self.open_settings();
                            }
                        });
                    });
                self.session_list(ui, ctx);
            });
    }

    fn sidebar_native_search(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let language = self.language;
        let palette = Palette::of(ui);
        if self.search.trim().is_empty()
            || self
                .filters
                .is_some_and(|provider| provider != Provider::Codex)
        {
            if self.native_search.pending || !self.native_search.hits.is_empty() {
                self.native_search.serial += 1;
                self.native_search.pending = false;
                self.native_search.hits.clear();
            }
            return;
        }
        ui.label(
            RichText::new(language.text(if self.remote.is_some() {
                "按标题和路径筛选"
            } else {
                "本地标题与路径筛选"
            }))
            .size(11.)
            .color(palette.muted),
        );
        ui.collapsing(language.text("Codex 原生全文搜索"), |ui| {
            let changed = ui
                .checkbox(
                    &mut self.native_search.archived,
                    language.text("包含已归档会话"),
                )
                .changed();
            if changed || self.native_search.query != self.search {
                self.native_search.serial += 1;
                self.native_search.pending = false;
                self.native_search.hits.clear();
                self.native_search.error = None;
                self.native_search.query = self.search.clone();
            }
            if ui
                .add_enabled(
                    !self.native_search.pending,
                    Button::new(language.text("全文搜索")),
                )
                .clicked()
            {
                self.native_search.serial += 1;
                let serial = self.native_search.serial;
                let query = self.search.clone();
                let archived = self.native_search.archived;
                self.native_search.query = query.clone();
                self.native_search.pending = true;
                self.native_search.error = None;
                self.background(ctx, move |workspace| {
                    Update::NativeSearch(
                        serial,
                        workspace.search(Provider::Codex, &query, archived),
                    )
                });
            }
            if self.native_search.pending {
                ui.label(
                    RichText::new(language.text("正在搜索…"))
                        .small()
                        .color(palette.muted),
                );
            }
            if let Some(error) = &self.native_search.error {
                ui.label(
                    RichText::new(language.text(error))
                        .small()
                        .color(palette.warn),
                );
            }
            let mut open = None;
            egui::ScrollArea::vertical()
                .id_salt("native search hits")
                .max_height(180.)
                .show(ui, |ui| {
                    for (index, hit) in self.native_search.hits.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            let row = &hit["thread"];
                            let title = row["name"]
                                .as_str()
                                .or(hit["snippet"].as_str())
                                .unwrap_or(language.text("未命名会话"));
                            if ui
                                .add(
                                    Button::new(title)
                                        .wrap()
                                        .min_size(vec2(ui.available_width(), 28.)),
                                )
                                .clicked()
                                && let Some(id) = row["id"].as_str()
                            {
                                open = Some(id.to_owned());
                            }
                            if let Some(snippet) =
                                hit["snippet"].as_str().filter(|snippet| *snippet != title)
                            {
                                ui.add(
                                    Label::new(RichText::new(snippet).small().color(palette.muted))
                                        .truncate(),
                                );
                            }
                        });
                    }
                });
            if let Some(id) = open
                && self.can_navigate_in_workspace()
            {
                let query = self.native_search.query.clone();
                self.begin_navigation();
                let epoch = self.epoch;
                self.view = View::new(Provider::Codex);
                self.view.id = Some(id.clone());
                self.action_dialog = None;
                self.note = None;
                self.load_models(ctx, Provider::Codex, false);
                self.restore_permission_mode(ctx);
                self.background(ctx, move |workspace| {
                    let result = workspace.history(Provider::Codex, &id, true).map(
                        |(session, mut history)| {
                            // A match is an optional landing aid. Failure to find it must
                            // not hide a successfully loaded session.
                            if let Ok(occurrence) = workspace.occurrences(&session, &query)
                                && let Some(turn) = occurrence["data"][0]["turnId"].as_str()
                                && let Some(index) = history
                                    .messages
                                    .iter()
                                    .position(|message| message.turn_id.as_deref() == Some(turn))
                            {
                                history.messages.drain(..index);
                            }
                            (session, history)
                        },
                    );
                    Update::Transcript(epoch, result, false)
                });
            }
        });
    }

    fn session_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        if let Some(warning) = Provider::ALL
            .into_iter()
            .filter(|provider| self.filters.is_none_or(|filter| filter == *provider))
            .find_map(|provider| {
                self.inventories
                    .get(&provider)
                    .and_then(|result| match result {
                        Ok(inventory) => inventory
                            .warning
                            .as_deref()
                            .map(|warning| language.text(warning).to_owned()),
                        Err(error) => {
                            Some(format!("{} · {}", provider.label(), language.text(error)))
                        }
                    })
            })
        {
            ui.add(egui::Label::new(RichText::new(warning).color(palette.muted).size(12.)).wrap());
        }
        let query = self.search.trim().to_lowercase();
        let catalog = self.codex_projects();
        let native: HashMap<_, _> = catalog
            .iter()
            .map(|project| (project.id.as_str(), project))
            .collect();
        let mut sessions: Vec<_> = self
            .inventories
            .iter()
            .filter(|(provider, _)| self.filters.is_none_or(|filter| filter == **provider))
            .filter_map(|(_, list)| list.as_ref().ok())
            .flat_map(|list| &list.sessions)
            .collect();
        sessions.sort_by_key(|row| std::cmp::Reverse(row.updated));
        let mut project_indices = HashMap::new();
        let mut projects = Vec::new();
        let mut project_names = HashMap::new();
        for row in sessions {
            let project = if row.provider == Provider::Codex {
                row.project_id
                    .as_deref()
                    .and_then(|id| native.get(id).copied())
            } else if self.filters.is_none() {
                let mut matches = catalog
                    .iter()
                    .filter(|project| project.roots.contains(&row.cwd));
                matches.next().filter(|_| matches.next().is_none())
            } else {
                None
            };
            if !query.is_empty()
                && !row.title.to_lowercase().contains(&query)
                && !row.cwd.to_string_lossy().to_lowercase().contains(&query)
                && !project.is_some_and(|project| project.name.to_lowercase().contains(&query))
            {
                continue;
            }
            let group_id = match project {
                Some(project) => ui.make_persistent_id(("codex project", &project.id)),
                None => ui.make_persistent_id(("project", &row.cwd)),
            };
            let index = *project_indices.entry(group_id).or_insert_with(|| {
                let path = project
                    .and_then(|project| project.roots.first())
                    .unwrap_or(&row.cwd);
                let name = project.map_or_else(
                    || {
                        path.file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy()
                            .into_owned()
                    },
                    |project| project.name.clone(),
                );
                *project_names.entry(name.clone()).or_insert(0) += 1;
                projects.push(SessionGroup {
                    key: group_id,
                    project_id: project.map(|project| project.id.clone()),
                    path: path.clone(),
                    name,
                    rows: Vec::new(),
                });
                projects.len() - 1
            });
            projects[index].rows.push(row.clone());
        }
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
                        run.provider.label(),
                        self.view.run.as_ref() == Some(&run.id),
                        0.,
                        true,
                    ) {
                        self.open_run(ctx, run);
                    }
                }
                if projects.is_empty() {
                    let text = if !self.inventory_pending.is_empty() {
                        language.text(if self.remote.is_some() {
                            "正在读取会话…"
                        } else {
                            "正在读取本机历史…"
                        })
                    } else if !query.is_empty() {
                        language.text("没有匹配的会话")
                    } else {
                        language.text(if self.remote.is_some() {
                            "暂无会话"
                        } else {
                            "暂无本机会话"
                        })
                    };
                    ui.label(RichText::new(text).color(palette.muted).size(13.));
                }
                for (
                    index,
                    SessionGroup {
                        key,
                        project_id,
                        path,
                        name,
                        rows,
                    },
                ) in projects.into_iter().enumerate()
                {
                    let project_name = if project_names[&name] > 1 {
                        if project_id.is_some() {
                            format!("{name} · {}", path.display())
                        } else {
                            path.display().to_string()
                        }
                    } else {
                        name
                    };
                    let active = rows.iter().any(|row| {
                        self.view.provider == row.provider && self.view.id.as_ref() == Some(&row.id)
                    });
                    let mut state =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ctx,
                            key,
                            index == 0 || active,
                        );
                    if !query.is_empty() {
                        state.set_open(true);
                    }
                    let openness = state.openness(ctx);
                    ui.push_id(key, |ui| {
                        let background = ui.painter().add(egui::Shape::Noop);
                        let row = ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 30.),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 2.;
                                ui.spacing_mut().button_padding.x = 5.;
                                let widgets = &mut ui.visuals_mut().widgets;
                                for visuals in [&mut widgets.hovered, &mut widgets.active] {
                                    visuals.weak_bg_fill = Color32::TRANSPARENT;
                                    visuals.bg_stroke = Stroke::NONE;
                                }
                                let new_session = Icon::Plus.button_layout(
                                    ui,
                                    &language.format(
                                        "在 {project} 新建会话",
                                        &[("project", &project_name)],
                                    ),
                                );
                                let header = Button::new((
                                    Icon::Right.atom(),
                                    Icon::Folder.atom(),
                                    RichText::new(&project_name).size(13.),
                                    Atom::grow(),
                                ))
                                .min_size(vec2(ui.available_width(), 30.))
                                .truncate()
                                .frame_when_inactive(false)
                                .atom_ui(ui);
                                Icon::paint_chevron_in(ui, &header, openness, palette.muted);
                                Icon::Folder.paint_in(ui, &header, palette.muted);
                                let header =
                                    header.response.on_hover_text(path.display().to_string());
                                (header, new_session)
                            },
                        );
                        let (header, new_session) = row.inner;
                        let row_hovered = ui.rect_contains_pointer(row.response.rect);
                        if row_hovered || new_session.response.has_focus() {
                            Icon::Plus.paint_in(ui, &new_session, palette.muted);
                        }
                        let new_session = new_session.response;
                        let focused = [&header, &new_session]
                            .into_iter()
                            .find(|button| button.has_focus());
                        let highlighted = row_hovered || focused.is_some();
                        let pressed = highlighted
                            && (header.is_pointer_button_down_on()
                                || new_session.is_pointer_button_down_on());
                        let hover_t =
                            ctx.animate_bool_with_time(key.with("hover"), highlighted, 0.12);
                        let press_t = ctx.animate_bool_with_time(key.with("press"), pressed, 0.08);
                        if hover_t > 0.005 {
                            let fill = lerp_color(palette.soft, palette.tint, press_t)
                                .gamma_multiply(hover_t);
                            let border = lerp_color(palette.line, palette.tint_line, press_t)
                                .gamma_multiply(hover_t);
                            ui.painter().set(
                                background,
                                egui::epaint::RectShape::new(
                                    row.response.rect,
                                    8,
                                    fill,
                                    Stroke::new(1., border),
                                    StrokeKind::Inside,
                                ),
                            );
                        }
                        if let Some(button) = focused {
                            ui.painter().rect_stroke(
                                button.rect.shrink(2.),
                                6,
                                Stroke::new(1., palette.accent),
                                StrokeKind::Inside,
                            );
                        }
                        if header.clicked() {
                            state.toggle(ui);
                        }
                        if new_session.clicked() {
                            state.set_open(true);
                            self.new_view(ctx, Some(path.clone()), project_id);
                        }
                    });
                    state.show_body_unindented(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.;
                        for row in rows {
                            let selected = self.view.provider == row.provider
                                && self.view.id.as_ref() == Some(&row.id);
                            if sidebar_row(
                                ui,
                                &row.title,
                                row.provider.label(),
                                selected,
                                17.,
                                false,
                            ) {
                                self.open(ctx, row);
                            }
                        }
                        ui.add_space(3.);
                    });
                }
            });
    }

    fn new_session_form(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        ui.label(RichText::new(language.text("新建会话")).size(31.));
        ui.add_space(16.);
        ui.add_enabled_ui(!self.view.pending, |ui| {
            ui.label(
                RichText::new(language.text(if self.remote.is_some() {
                    "目标电脑的工作目录"
                } else {
                    "工作目录"
                }))
                .color(palette.muted)
                .size(12.5),
            );
            if self.remote.is_some() {
                let mut directory = self.folder.to_string_lossy().into_owned();
                if input::text_edit(
                    ui,
                    TextEdit::singleline(&mut directory)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(9, 7))
                        .hint_text(language.text("目标电脑的工作目录")),
                    false,
                )
                .changed()
                {
                    self.folder = PathBuf::from(directory);
                    self.view.project_id = None;
                    self.view.clear_startup_settings();
                }
            } else {
                let path = self.folder.display().to_string();
                let name = self.folder.file_name().map(|name| name.to_string_lossy());
                let project = self
                    .view
                    .project_id
                    .as_deref()
                    .and_then(|id| self.codex_project(id));
                let title = project
                    .map(|project| project.name.as_str())
                    .or(name.as_deref())
                    .unwrap_or(if path.is_empty() {
                        language.text("选择文件夹")
                    } else {
                        &path
                    });
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 60.), egui::Sense::click());
                response.widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::Button,
                        response.enabled(),
                        language.text("选择工作目录"),
                    )
                });
                let visuals = ui.style().interact(&response);
                let painter = ui.painter();
                painter.rect(
                    rect,
                    10,
                    visuals.weak_bg_fill,
                    visuals.bg_stroke,
                    StrokeKind::Inside,
                );
                let badge = Rect::from_center_size(
                    egui::pos2(rect.left() + 29., rect.center().y),
                    vec2(32., 32.),
                );
                painter.rect_filled(badge, 8, palette.tint);
                Icon::Folder.paint(painter, badge.center(), palette.accent);
                Icon::Right.paint(painter, rect.right_center() - vec2(17., 0.), palette.muted);
                let x = rect.left() + 53.;
                let width = (rect.width() - 85.).max(1.);
                let show_path = !path.is_empty() && title != path;
                if show_path {
                    paint_line(
                        ui,
                        egui::pos2(x, rect.top() + 31.),
                        &path,
                        11.5,
                        palette.muted,
                        width,
                    );
                }
                let top = rect.top() + if show_path { 10. } else { 18. };
                paint_line(ui, egui::pos2(x, top), title, 14., palette.ink, width);
                if response
                    .on_hover_text(if path.is_empty() {
                        language.text("选择工作目录")
                    } else {
                        path.as_str()
                    })
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title(language.text("选择 Naro 的工作目录"))
                        .pick_folder()
                {
                    if !project.is_some_and(|project| {
                        project.roots.iter().any(|root| path.starts_with(root))
                    }) {
                        self.view.project_id = None;
                    }
                    self.view.clear_startup_settings();
                    self.folder = path;
                }
            }

            ui.add_space(12.);
            ui.label(
                RichText::new(language.text("引擎"))
                    .color(palette.muted)
                    .size(12.5),
            );
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 2. * ui.spacing().item_spacing.x) / 3.;
                for provider in Provider::ALL {
                    let active = self.view.provider == provider;
                    let button = ui.add(
                        Button::new(RichText::new(provider.label()).color(palette.ink))
                            .min_size(vec2(width, 39.))
                            .selected(active),
                    );
                    if button.clicked() && !active {
                        self.view.clear_startup_settings();
                        self.view.provider = provider;
                        self.view.project_id = None;
                        self.restore_model_preference();
                        self.restore_permission_mode(ctx);
                        self.view.collaboration_mode = None;
                        self.load_models(ctx, provider, false);
                    }
                }
            });
            ui.add_space(10.);
            egui::CollapsingHeader::new(language.text("更多设置"))
                .id_salt(("startup settings", self.epoch, self.view.provider))
                .default_open(false)
                .show(ui, |ui| {
                    startup_settings(ui, &mut self.view.startup, self.view.provider, true)
                });
        });
    }

    fn conversation(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let cwd = self
            .view
            .session
            .as_ref()
            .map_or(self.folder.as_path(), |session| session.cwd.as_path());
        if let Some(warning) = &self.view.transcript.warning {
            ui.label(
                RichText::new(language.text(warning))
                    .color(palette.warn)
                    .size(13.),
            );
            if self.view.transcript.messages.is_empty()
                && let Some(session) = &self.view.session
                && let Some(preview) = &session.preview
            {
                render_prompt(ui, preview);
            }
        }
        let shown = self.view.transcript.messages.len().saturating_sub(300);
        if shown > 0 {
            ui.label(
                RichText::new(language.text("显示本会话最近 300 条消息。"))
                    .color(palette.muted)
                    .size(12.),
            );
        }
        for (index, entry) in self.view.transcript.messages.iter().enumerate().skip(shown) {
            ui.push_id(("message", index), |ui| match entry.role {
                Role::User => {
                    self.thumbnails.show(
                        ui,
                        &history_images(&entry.attachments, self.remote.as_ref()),
                    );
                    render_prompt_files(ui, &entry.local_files);
                    render_prompt(ui, &entry.text);
                }
                Role::Assistant => {
                    let file_links = ReplyFileLinks::new(cwd, self.remote.as_ref(), &entry.work);
                    for step in entry
                        .work
                        .steps
                        .iter()
                        .filter(|step| step.kind == StepKind::Note)
                    {
                        render_progress_note(
                            ui,
                            &mut self.markdown,
                            &mut self.files,
                            &step.id,
                            &step.text,
                            file_links,
                        );
                    }
                    render_reply(
                        ui,
                        &mut self.markdown,
                        &mut self.files,
                        &entry.text,
                        &entry.work,
                        file_links,
                    );
                    if let Some(client) = &self.remote {
                        self.thumbnails.show(
                            ui,
                            &history_images(&entry.work.remote_previews, Some(client)),
                        );
                    }
                }
            });
            ui.add_space(10.);
        }
        self.render_live(ui);
    }

    fn render_live(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let Some(snapshot) = &self.view.snapshot else {
            return;
        };
        let remote = self
            .remote
            .as_ref()
            .map(|client| (client, snapshot.run.id.as_str()));
        let cwd = self
            .view
            .session
            .as_ref()
            .map_or(self.folder.as_path(), |session| session.cwd.as_path());
        let replies = snapshot.replies();
        let run = if remote.is_none() {
            self.service.run(&snapshot.run.id).ok()
        } else {
            None
        };
        let follow_ups = snapshot
            .follow_ups
            .iter()
            .filter(|entry| entry.delivery == Delivery::Sent)
            .map(|entry| {
                (
                    Some(entry.id.as_str()),
                    entry.prompt.as_str(),
                    entry.images.as_slice(),
                    entry.local_files.as_slice(),
                    entry.image_start,
                )
            });
        let first = (
            None,
            snapshot.prompt.as_str(),
            snapshot.images.as_slice(),
            snapshot.local_files.as_slice(),
            0,
        );
        for (after, prompt, images, files, image_start) in std::iter::once(first).chain(follow_ups)
        {
            ui.push_id((&snapshot.run.id, after), |ui| {
                self.thumbnails
                    .show(ui, &run_images(run.as_deref(), remote, images, image_start));
                render_prompt_files(ui, files);
                render_prompt(ui, prompt);
                for reply in &replies {
                    let file_links = ReplyFileLinks::new(cwd, self.remote.as_ref(), reply.work);
                    for step in reply.work.steps.iter().filter(|step| {
                        step.kind == StepKind::Note && step.after.as_deref() == after
                    }) {
                        render_progress_note(
                            ui,
                            &mut self.markdown,
                            &mut self.files,
                            &step.id,
                            &step.text,
                            file_links,
                        );
                    }
                }
                if let Some(reply) = replies.iter().find(|reply| reply.after == after) {
                    render_reply(
                        ui,
                        &mut self.markdown,
                        &mut self.files,
                        &reply.text,
                        reply.work,
                        ReplyFileLinks::new(cwd, self.remote.as_ref(), reply.work),
                    );
                    if let Some((client, _)) = remote {
                        self.thumbnails.show(
                            ui,
                            &history_images(&reply.work.remote_previews, Some(client)),
                        );
                    }
                }
            });
        }
        if snapshot.status == Status::Running {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.).color(palette.accent));
                ui.label(
                    RichText::new(language.text(&snapshot.activity))
                        .color(palette.muted)
                        .size(13.),
                );
            });
        } else if snapshot.status == Status::Stopped {
            ui.label(
                RichText::new(language.text("已停止"))
                    .color(palette.muted)
                    .size(13.),
            );
        }
        if let Some(error) = &snapshot.error {
            ui.label(RichText::new(language.text(error)).color(palette.warn));
        }
        if let Some(approval) = &snapshot.approval {
            let mut decision = None;
            surface(palette, palette.notice, Margin::same(13)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(language.format(
                        "确认操作 · {title}",
                        &[("title", language.text(&approval.title))],
                    ))
                    .strong(),
                );
                if let Some(mode) = &snapshot.active_permission_mode {
                    ui.label(
                        RichText::new(language.format(
                            "当前回复使用{mode}。",
                            &[("mode", language.text(&mode.label))],
                        ))
                        .color(palette.muted)
                        .size(12.),
                    );
                }
                egui::ScrollArea::vertical()
                    .id_salt(&approval.id)
                    .max_height(150.)
                    .show(ui, |ui| {
                        ui.label(RichText::new(&approval.details).monospace());
                    });

                ui.horizontal(|ui| {
                    for (approved, label) in [
                        (true, language.text("允许一次")),
                        (false, language.text("拒绝")),
                    ] {
                        if ui
                            .add_enabled(!self.run_command_pending, Button::new(label))
                            .clicked()
                        {
                            decision = Some(approved);
                        }
                    }
                });
            });
            if let Some(allow) = decision {
                self.run_command(
                    ui.ctx(),
                    snapshot.run.id.clone(),
                    RunCommand::Approve {
                        approval_id: approval.id.clone(),
                        allow,
                    },
                );
            }
        }
    }

    fn follow_up_queue(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        enum Action {
            Withdraw,
            Steer,
        }

        let language = Language::in_ui(ui);
        let Some(snapshot) = &self.view.snapshot else {
            return;
        };
        let entries = snapshot
            .follow_ups
            .iter()
            .filter(|entry| entry.delivery != Delivery::Sent)
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return;
        }
        let run_id = &snapshot.run.id;
        let remote = self.remote.as_ref().map(|client| (client, run_id.as_str()));
        let run = if remote.is_none() {
            self.service.run(run_id).ok()
        } else {
            None
        };
        let running = snapshot.status == Status::Running && !self.view.stopping;
        let can_steer = snapshot.run.provider.supports_steering();
        let mut action = None;
        surface(palette, palette.soft, Margin::symmetric(11, 7))
            .corner_radius(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(language.text("发送队列"))
                        .size(12.)
                        .strong()
                        .color(palette.muted),
                );
                egui::ScrollArea::vertical()
                    .id_salt(("follow-up queue", run_id))
                    .max_height(135.)
                    .show(ui, |ui| {
                        for entry in entries {
                            ui.push_id(&entry.id, |ui| {
                                ui.separator();
                                let queued = entry.mode == SendMode::Queue
                                    && entry.delivery == Delivery::Pending;
                                ui.horizontal(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if running && queued {
                                            if ui
                                                .add_enabled(
                                                    !self.view.pending && !self.run_command_pending,
                                                    Button::new(language.text("撤回修改")).small(),
                                                )
                                                .clicked()
                                            {
                                                action =
                                                    Some((entry.id.as_str(), Action::Withdraw));
                                            }
                                            if can_steer
                                                && ui
                                                    .add_enabled(
                                                        !self.run_command_pending,
                                                        Button::new(
                                                            language.text(SendMode::Steer.label()),
                                                        )
                                                        .small(),
                                                    )
                                                    .clicked()
                                            {
                                                action = Some((entry.id.as_str(), Action::Steer));
                                            }
                                        }
                                        ui.vertical(|ui| {
                                            ui.set_width(ui.available_width());
                                            let preview = engine::cut(&entry.prompt, 320)
                                                .split_whitespace()
                                                .collect::<Vec<_>>()
                                                .join(" ");
                                            ui.add(Label::new(preview).truncate())
                                                .on_hover_text(engine::cut(&entry.prompt, 1200));
                                            let status = match (entry.delivery, entry.mode) {
                                                (Delivery::Unsent, SendMode::Steer) => "引导未确认",
                                                (Delivery::Unsent, SendMode::Queue) => "未发送",
                                                (_, SendMode::Queue) => "队列中",
                                                (_, SendMode::Steer) => "正在引导",
                                            };
                                            let mut details = language.text(status).to_owned();
                                            for name in
                                                entry.local_files.iter().map(LocalAttachment::name)
                                            {
                                                details.push_str(" · ");
                                                details.push_str(name);
                                            }
                                            if let Some(error) = &entry.error {
                                                details.push_str(" · ");
                                                details.push_str(language.text(error));
                                            }
                                            let color = if entry.error.is_some() {
                                                palette.warn
                                            } else {
                                                palette.muted
                                            };
                                            ui.add(
                                                Label::new(
                                                    RichText::new(&details).size(11.).color(color),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(details);
                                        });
                                    });
                                });
                                self.thumbnails.show(
                                    ui,
                                    &run_images(
                                        run.as_deref(),
                                        remote,
                                        &entry.images,
                                        entry.image_start,
                                    ),
                                );
                            });
                        }
                    });
            });
        ui.add_space(5.);
        if let Some((message_id, action)) = action {
            let command = match action {
                Action::Steer => RunCommand::Steer(message_id.to_owned()),
                Action::Withdraw => RunCommand::Withdraw(message_id.to_owned()),
            };
            self.run_command(ui.ctx(), run_id.clone(), command);
        }
    }

    fn composer(&mut self, ui: &mut Ui, ctx: &egui::Context, editor_max_height: f32) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let editor_id = ui.make_persistent_id(("prompt", self.epoch));
        self.composer_focus = Some(editor_id);
        self.follow_up_queue(ui);
        if self.restore_composer_focus && self.action_dialog.is_none() {
            ctx.memory_mut(|memory| memory.request_focus(editor_id));
            self.restore_composer_focus = false;
        }
        let keyboard_enabled = self.action_dialog.is_none()
            && !self.pair_dialog
            && !self.quota_dialog
            && !self.thumbnails.is_preview_open()
            && self
                .view
                .question_form
                .as_ref()
                .is_none_or(|form| !form.open);
        if keyboard_enabled {
            self.clipboard.editor.set(Some(editor_id));
        }
        for image in self.clipboard.take_images(editor_id) {
            self.add_clipboard_image(image);
        }
        let focused = keyboard_enabled && ctx.memory(|memory| memory.has_focus(editor_id));
        let send_shortcut = if focused {
            ctx.input_mut(|input| {
                input::send_shortcut(&mut input.events, &mut self.view.ime_composing)
            })
        } else {
            self.view.ime_composing = false;
            false
        };
        let frame = surface(palette, palette.paper, Margin::symmetric(11, 9))
            .corner_radius(14)
            .stroke(Stroke::new(1., palette.composer_border));
        frame.show(ui, |ui| {
            let busy = self.view.busy();
            let running = self.view.run.is_some();
            if self.view.editing_withdrawn || !self.view.other_drafts.is_empty() {
                ui.horizontal(|ui| {
                    if self.view.editing_withdrawn {
                        ui.label(
                            RichText::new(language.text("正在修改已撤回的消息"))
                                .color(palette.muted)
                                .size(12.),
                        );
                    }
                    if !self.view.other_drafts.is_empty()
                        && ui
                            .add_enabled(
                                !self.view.pending,
                                Button::new(language.text("切换草稿")).small(),
                            )
                            .clicked()
                        && let Some(draft) = self.view.other_drafts.pop_front()
                    {
                        self.view.show_draft(draft);
                        ctx.memory_mut(|memory| memory.request_focus(editor_id));
                    }
                });
            }
            if let Some(form) = &mut self.view.question_form
                && !form.open
                && ui.button(language.text("回答问题")).clicked()
            {
                form.open = true;
            }
            self.draft_attachments(ui);
            self.compose_editor(ui, editor_id, editor_max_height);
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 34.),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.scope(|ui| {
                        let spacing = ui.spacing_mut();
                        spacing.item_spacing.x = 2.;
                        spacing.button_padding.x = 8.;
                        self.composer_attachments(ui, editor_id);
                        self.permission_settings(ui, busy);
                        if !self.view.is_new() && !running {
                            let trigger = composer_tool(
                                ui,
                                Icon::Sliders,
                                language.text("更多设置"),
                                palette.muted,
                            );
                            egui::Popup::menu(&trigger)
                                .align(egui::RectAlign::TOP_START)
                                .frame(popup(palette, Margin::same(12)))
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| {
                                    ui.set_width(300.);
                                    ui.add_enabled_ui(!self.view.pending, |ui| {
                                        startup_settings(
                                            ui,
                                            &mut self.view.startup,
                                            self.view.provider,
                                            false,
                                        )
                                    });
                                });
                        }
                    });

                    let has_draft = self.view.has_draft();
                    let ready = !self.view.pending
                        && !self.native_pending()
                        && !self.view.stopping
                        && !self.permission_pending
                        && !self.run_command_pending
                        && self.remote_error.is_none()
                        && !self.view.ime_composing
                        && has_draft
                        && (running
                            || self.view.id.is_some()
                            || if self.remote.is_some() {
                                !self.folder.to_string_lossy().trim().is_empty()
                            } else {
                                self.folder.is_dir()
                            });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if running && !has_draft && !self.view.pending {
                            self.compose_stop(ui);
                        } else {
                            self.compose_send(ui, ctx, ready, send_shortcut);
                        }
                        let model_width = ui.available_width().clamp(68., 285.);
                        ui.allocate_ui_with_layout(
                            vec2(model_width, 34.),
                            Layout::right_to_left(Align::Center),
                            |ui| self.model_settings(ui, ctx),
                        );
                    });
                },
            );
            for note in [
                self.remote_error
                    .as_deref()
                    .or_else(|| self.view.run.as_ref().and(self.snapshot_error.as_deref())),
                self.note.as_deref(),
                self.model_errors
                    .get(&self.view.provider)
                    .map(String::as_str),
            ]
            .into_iter()
            .flatten()
            {
                ui.add(
                    Label::new(
                        RichText::new(language.text(note))
                            .color(palette.warn)
                            .size(12.),
                    )
                    .truncate(),
                )
                .on_hover_text(note);
            }
        });
    }

    fn compose_editor(&mut self, ui: &mut Ui, id: egui::Id, max_height: f32) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let mut pasted_image = None;
        let editor = TextEdit::multiline(&mut self.view.draft)
            .hint_text(RichText::new(language.text("输入消息…")).color(palette.placeholder))
            .text_color(palette.composer_text)
            .desired_rows(COMPOSER_MIN_EDITOR_ROWS)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(5, 3))
            .return_key(egui::KeyboardShortcut::new(
                egui::Modifiers::SHIFT,
                egui::Key::Enter,
            ))
            .frame(false);
        let composing = self.view.ime_composing;
        composer_input(ui, id, max_height, composing, |ui| {
            let output = input::text_edit_with_menu(ui, id, editor, false, || {
                pasted_image = input::read_clipboard_image();
                pasted_image.is_some()
            });
            if composing && let Some(range) = output.cursor_range.filter(|range| !range.is_empty())
            {
                let [start, end] = range
                    .sorted_cursors()
                    .map(|cursor| output.galley.layout_from_cursor(cursor));
                let painter = ui.painter().with_clip_rect(output.text_clip_rect);
                for index in start.row..=end.row {
                    let row = &output.galley.rows[index];
                    let left = if index == start.row {
                        row.x_offset(start.column)
                    } else {
                        0.
                    };
                    let right = if index == end.row {
                        row.x_offset(end.column)
                    } else {
                        row.size.x
                    };
                    let origin = output.galley_pos + vec2(row.pos.x, row.rect().bottom() - 1.);
                    if left < right {
                        painter.hline(
                            (origin.x + left)..=(origin.x + right),
                            origin.y,
                            Stroke::new(1., palette.composer_text),
                        );
                    }
                }
            }
        });
        if let Some(image) = pasted_image {
            self.add_clipboard_image(image);
        }
    }

    fn composer_attachments(&mut self, ui: &mut Ui, editor_id: egui::Id) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let title = language.text(if self.remote.is_some() {
            "添加图片"
        } else {
            "添加文件或文件夹"
        });
        let trigger = ui
            .add_enabled_ui(!self.view.pending, |ui| {
                composer_tool(ui, Icon::Plus, title, palette.composer_text)
            })
            .inner;
        if !trigger.clicked() {
            return;
        }
        if self.remote.is_some() {
            if let Some(paths) = rfd::FileDialog::new()
                .set_title(title)
                .add_filter(title, &["png", "jpg", "jpeg", "gif", "webp"])
                .pick_files()
            {
                self.accept_attachments(ui.ctx(), editor_id, paths);
            }
            return;
        }
        let directory = self
            .view
            .session
            .as_ref()
            .map_or(&self.folder, |session| &session.cwd);
        #[cfg(target_os = "macos")]
        if let Some(paths) = macos::pick_attachments(directory, title, language.text("添加")) {
            self.accept_attachments(ui.ctx(), editor_id, paths);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut dialog =
                egui_file_dialog::FileDialog::new().initial_directory(directory.clone());
            dialog.labels_mut().title_select_multiple = title.to_owned();
            dialog.labels_mut().cancel = language.text("取消").to_owned();
            dialog.pick_multiple();
            self.attachment_picker = Some((editor_id, dialog));
        }
    }

    fn accept_attachments(
        &mut self,
        ctx: &egui::Context,
        editor_id: egui::Id,
        paths: Vec<PathBuf>,
    ) {
        self.add_files(paths);
        ctx.memory_mut(|memory| memory.request_focus(editor_id));
        ctx.request_repaint();
    }

    fn compose_send(&mut self, ui: &mut Ui, ctx: &egui::Context, ready: bool, send_shortcut: bool) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let label = if self.view.pending {
            language.text("发送中")
        } else {
            language.text("发送")
        };
        let ready_t = ctx.animate_bool_with_time(ui.id().with("send_ready"), ready, 0.14);
        let fill = lerp_color(palette.send_disabled, palette.composer_text, ready_t);
        let text = lerp_color(palette.send_disabled_text, palette.paper, ready_t);
        let action = solid(ui, fill, palette.send_hover, |ui| {
            ui.visuals_mut().disabled_alpha = 1.;
            ui.add_enabled_ui(ready, |ui| {
                let button = Icon::Send
                    .label(RichText::new(label).color(text))
                    .min_size(vec2(84., 34.))
                    .atom_ui(ui);
                Icon::Send.paint_in(ui, &button, text);
                button.response
            })
            .inner
        });
        if action
            .on_hover_text(language.text("Enter 发送 · Shift+Enter 换行"))
            .clicked()
            || (send_shortcut && ready)
        {
            self.submit(ctx);
        }
    }

    fn compose_stop(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let label = if self.view.stopping {
            language.text("停止中")
        } else {
            language.text("停止")
        };
        let action = solid(ui, palette.danger_surface, palette.danger_hover, |ui| {
            ui.add_enabled_ui(!self.view.stopping && !self.run_command_pending, |ui| {
                let button = Icon::Stop
                    .label(RichText::new(label).color(palette.danger))
                    .min_size(vec2(84., 34.))
                    .atom_ui(ui);
                Icon::Stop.paint_in(ui, &button, palette.danger);
                button.response
            })
            .inner
        });
        if action.clicked()
            && let Some(id) = &self.view.run
        {
            self.run_command(ui.ctx(), id.clone(), RunCommand::Stop);
        }
    }

    fn model_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let provider = self.view.provider;
        let catalog = self.models.get(&provider);
        let choices = catalog.map_or(&[][..], |catalog| catalog.models.as_slice());
        let model_id = self.view.selected_model(catalog).map(str::to_owned);
        let pending = self.model_pending.contains(&provider);
        let selected = choices
            .iter()
            .find(|model| Some(&model.id) == model_id.as_ref());
        let fast_model = selected.filter(|model| model.fast_mode);
        let speed_button_width = 36.;
        let mut summary = vec![
            selected
                .map(|model| model.label.as_str())
                .or(model_id.as_deref())
                .unwrap_or(if pending {
                    language.text("正在读取模型…")
                } else {
                    language.text("选择模型")
                }),
        ];
        if let Some(effort) = selected.and_then(|model| {
            model.reasoning_choice(self.view.selection.reasoning_effort.as_deref())
        }) {
            summary.push(language.text(&effort.label));
        }
        if self.view.collaboration_mode == Some(CollaborationMode::Plan) {
            summary.push(language.text(CollaborationMode::Plan.label()));
        }
        let caption = summary.join(" · ");
        let description = language.format("模型设置 · {summary}", &[("summary", &caption)]);
        let mut atoms = egui::Atoms::new(RichText::new(&caption).color(palette.muted).size(12.5));
        atoms.push_right(Icon::Down.atom());

        let mut trigger_width = ui.available_width();
        if fast_model.is_some() {
            let speed_button_gap = 2.;
            ui.spacing_mut().item_spacing.x = speed_button_gap;
            trigger_width -= speed_button_width + speed_button_gap;
        }
        let trigger = ui
            .allocate_ui(vec2(trigger_width.max(0.), 33.), |ui| {
                let button = Button::new(atoms)
                    .frame_when_inactive(false)
                    .truncate()
                    .min_size(vec2(0., 33.))
                    .atom_ui(ui);
                Icon::Down.paint_in(ui, &button, palette.muted);
                button.response
            })
            .inner;
        trigger.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, trigger.enabled(), &description)
        });
        let trigger = trigger.on_hover_text(description);

        let mut remember_selection = false;
        if let Some(model) = fast_model {
            let fast = model
                .speed_choice(self.view.selection.fast_mode)
                .unwrap_or(false);
            let icon = if fast {
                Icon::LightningFilled
            } else {
                Icon::Lightning
            };
            let button = Button::new(icon.atom())
                .frame_when_inactive(false)
                .stroke(Stroke::NONE)
                .min_size(vec2(speed_button_width, 33.))
                .atom_ui(ui);
            icon.paint_in(
                ui,
                &button,
                if fast { palette.accent } else { palette.muted },
            );
            let mut response = button.response;
            let fast = if response.clicked() {
                self.view.selection.model = Some(model.id.clone());
                self.view.selection.fast_mode = Some(!fast);
                remember_selection = true;
                response.mark_changed();
                !fast
            } else {
                fast
            };
            response.widget_info(|| {
                WidgetInfo::selected(
                    WidgetType::Checkbox,
                    response.enabled(),
                    fast,
                    language.text("快速模式"),
                )
            });
            response.on_hover_text(if fast {
                language.text("快速模式")
            } else {
                language.text("标准速度")
            });
        }
        let mut refresh = false;
        let panel = egui::Popup::menu(&trigger);
        let opening = trigger.clicked() && !panel.is_open();
        panel
            .align(egui::RectAlign::TOP_END)
            .gap(7.)
            .frame(popup(palette, Margin::same(12)))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(285.);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(language.text("模型设置")).strong().size(15.));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        refresh = ui
                            .add_enabled_ui(!pending, |ui| {
                                Icon::refresh_button(
                                    ui,
                                    language.text("刷新模型"),
                                    palette.muted,
                                    pending,
                                )
                            })
                            .inner
                            .clicked();
                        if !pending && let Some(problem) = self.model_errors.get(&provider) {
                            let message = language.text(problem);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(message).color(palette.warn).size(12.),
                                )
                                .truncate(),
                            )
                            .on_hover_text(message);
                        }
                    });
                });
                ui.label(
                    RichText::new(language.text("模型"))
                        .color(palette.muted)
                        .size(12.),
                );
                egui::ScrollArea::vertical()
                    .max_height(154.)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if selected.is_none()
                            && let Some(id) = &model_id
                        {
                            model_row(ui, id, true);
                        }
                        for model in choices {
                            let active = model_id.as_ref() == Some(&model.id);
                            if model_row(ui, &model.label, active) {
                                if !active {
                                    self.view.selection = ModelSelection::default();
                                }
                                self.view.selection.model = Some(model.id.clone());
                                remember_selection = true;
                            }
                        }
                    });
                if let Some(catalog) = catalog
                    && !catalog.collaboration_modes.is_empty()
                {
                    ui.label(
                        RichText::new(language.text("协作模式"))
                            .color(palette.muted)
                            .size(12.),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for &mode in &catalog.collaboration_modes {
                            let active = self
                                .view
                                .collaboration_mode
                                .unwrap_or(CollaborationMode::Default)
                                == mode;
                            if ui
                                .selectable_label(active, language.text(mode.label()))
                                .clicked()
                                && !active
                            {
                                self.view.collaboration_mode = Some(mode);
                            }
                        }
                    });
                }
                if let Some(model) = choices
                    .iter()
                    .find(|model| Some(model.id.as_str()) == self.view.selected_model(catalog))
                {
                    if !model.reasoning_efforts.is_empty() {
                        let selected =
                            model.reasoning_choice(self.view.selection.reasoning_effort.as_deref());
                        if let Some(effort) = effort_track(ui, &model.reasoning_efforts, selected) {
                            self.view.selection.reasoning_effort = Some(effort.id.clone());
                            self.view.selection.model = Some(model.id.clone());
                            remember_selection = true;
                        }
                    }
                }
            });
        if remember_selection {
            self.model_preferences
                .insert(provider, self.view.selection.clone());
        }
        if opening || refresh {
            self.load_models(ctx, provider, refresh);
        }
    }

    fn permission_settings(&mut self, ui: &mut Ui, busy: bool) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let provider = self.view.provider;
        let Some(catalog) = self
            .models
            .get(&provider)
            .filter(|catalog| !catalog.permission_modes.is_empty())
        else {
            return;
        };
        let category = if provider == Provider::Antigravity {
            language.text("CLI 执行模式")
        } else {
            language.text("权限")
        };
        let choices = &catalog.permission_modes;
        let default = catalog.default_permission_mode;
        let selected = self.view.selected_permission(Some(catalog)).or(default);
        let permission = choices
            .iter()
            .find(|mode| Some(mode.id.as_str()) == selected);
        let running = self
            .view
            .snapshot
            .as_ref()
            .filter(|snapshot| snapshot.status == Status::Running)
            .and_then(|snapshot| snapshot.active_permission_mode.as_ref());
        let running_label = running.map(|mode| {
            language.format(
                "当前回复使用{mode}。",
                &[("mode", language.text(&mode.label))],
            )
        });
        let follows_session = default.is_none() && provider != Provider::Codex;
        let description = if let Some(label) = &running_label {
            label.clone()
        } else if let Some(mode) = permission {
            language.format(
                "{category}：{mode}",
                &[("category", category), ("mode", language.text(&mode.label))],
            )
        } else if follows_session && selected.is_none() {
            language.format(
                "{category}：{mode}",
                &[("category", category), ("mode", language.text("跟随会话"))],
            )
        } else {
            category.to_owned()
        };
        let icon = running
            .or(permission)
            .map_or(Icon::Lock, |mode| Icon::permission(&mode.id));
        let color = if matches!(icon, Icon::Unlock) {
            palette.danger
        } else {
            palette.composer_text
        };
        let trigger = composer_tool(ui, icon, &description, color);
        let mut selection = None;
        egui::Popup::menu(&trigger)
            .align(egui::RectAlign::TOP_START)
            .gap(7.)
            .frame(popup(palette, Margin::same(12)))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(228.);
                ui.label(RichText::new(category).strong().size(15.));
                if let Some(label) = &running_label {
                    ui.label(RichText::new(label).color(palette.muted).size(12.));
                }
                if busy {
                    ui.label(
                        RichText::new(language.text("下条开启新回复的消息使用此设置。"))
                            .color(palette.muted)
                            .size(12.),
                    );
                }
                ui.add_enabled_ui(!self.view.pending && !self.permission_pending, |ui| {
                    if follows_session
                        && model_row(ui, language.text("跟随会话"), selected.is_none())
                    {
                        selection = Some(None);
                    }
                    for mode in choices {
                        if model_row(
                            ui,
                            language.text(&mode.label),
                            selected == Some(mode.id.as_str()),
                        ) {
                            selection = Some(Some(mode.id.clone()));
                        }
                    }
                });
                if let Some(mode) = permission
                    && !mode.description.is_empty()
                {
                    ui.label(
                        RichText::new(language.text(&mode.description))
                            .color(palette.muted)
                            .size(12.),
                    );
                }
            });
        if let Some(mode) = selection {
            self.save_permission_mode(ui.ctx(), mode);
        }
    }

    fn draft_attachments(&mut self, ui: &mut Ui) {
        let language = Language::in_ui(ui);
        if self.view.pending
            || (self.view.attachments.is_empty() && self.view.local_files.is_empty())
        {
            return;
        }
        let mut remove_image = None;
        let mut remove_file = None;
        egui::ScrollArea::vertical()
            .id_salt("draft attachments")
            .min_scrolled_height(76.)
            .max_height(76.)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for image in &self.view.attachments {
                        if ui
                            .push_id(image.id(), |ui| {
                                ui.add(egui::Button::new(format!("{}  ×", image.name())).small())
                                    .on_hover_text(language.format(
                                        "{width} × {height} · 点击移除",
                                        &[
                                            ("width", &image.width().to_string()),
                                            ("height", &image.height().to_string()),
                                        ],
                                    ))
                                    .clicked()
                            })
                            .inner
                        {
                            remove_image = Some(image.id().to_owned());
                        }
                    }
                    for (index, file) in self.view.local_files.iter().enumerate() {
                        if local_file_chip(ui, file, true).clicked() {
                            remove_file = Some(index);
                        }
                    }
                });
            });
        if let Some(id) = remove_image {
            self.view.attachments.retain(|image| image.id() != id);
        }
        if let Some(index) = remove_file {
            self.view.local_files.remove(index);
        }
    }

    fn settings(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        let language = self.language;
        let appearance = self.appearance;
        let mut back = false;
        window_header(
            ctx,
            "settings-header",
            WINDOW_BUTTONS_INSET,
            Layout::left_to_right(Align::Center),
            |ui| {
                back = ui.button(language.text("返回会话")).clicked();
                ui.label(RichText::new(language.text("设置")).size(13.));
                window_drag_area(ui, ui.available_rect_before_wrap());
            },
        );
        let Some(page) = &mut self.settings_page else {
            return;
        };
        let mut save = false;
        let mut save_paths = false;
        let mut restart = false;
        let has_update = self.updates.has_update();
        egui::SidePanel::left("settings-tabs")
            .exact_width(196.)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(Margin::symmetric(13, 16)),
            )
            .show(ctx, |ui| {
                let width = ui.available_width();
                for choice in SettingsTab::ALL {
                    let selected = page.tab == choice;
                    let button = Button::new((
                        RichText::new(language.text(choice.label()))
                            .size(13.5)
                            .color(if selected {
                                palette.accent
                            } else {
                                palette.ink
                            }),
                        egui::Atom::grow(),
                    ))
                    .min_size(vec2(width, 34.))
                    .selected(selected)
                    .frame_when_inactive(selected);
                    let response = ui.add(button);
                    if choice == SettingsTab::Updates && has_update {
                        ui.painter().circle_filled(
                            response.rect.right_center() - vec2(10., 0.),
                            2.5,
                            palette.accent,
                        );
                    }
                    if response.clicked() {
                        page.tab = choice;
                    }
                    ui.add_space(2.);
                }
                let rect = ui.max_rect();
                ui.painter().vline(
                    rect.right() + 13. - 0.5,
                    ui.ctx().content_rect().y_range(),
                    Stroke::new(1., palette.line),
                );
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.canvas))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("settings page", page.tab))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(24.);
                        centered(ui, 600., |ui| match page.tab {
                            SettingsTab::General => {
                                settings_group(ui, language.text("通用"), |ui| {
                                    ui.label(language.text("界面语言"));
                                    egui::ComboBox::from_id_salt("interface language")
                                        .width(160.)
                                        .selected_text(self.language.name())
                                        .show_ui(ui, |ui| {
                                            for choice in Language::ALL {
                                                ui.selectable_value(
                                                    &mut self.language,
                                                    choice,
                                                    choice.name(),
                                                );
                                            }
                                        });
                                    ui.add_space(12.);
                                    ui.label(language.text("外观"));
                                    egui::ComboBox::from_id_salt("appearance")
                                        .width(160.)
                                        .selected_text(language.text(self.appearance.label()))
                                        .show_ui(ui, |ui| {
                                            for choice in Appearance::ALL {
                                                ui.selectable_value(
                                                    &mut self.appearance,
                                                    choice,
                                                    language.text(choice.label()),
                                                );
                                            }
                                        });
                                    ui.label(RichText::new(language.text(
                                        "跟随当前设备的系统外观，切换后立即生效。",
                                    )).color(palette.muted).size(12.));
                                });
                            }
                            SettingsTab::Session => {
                                settings_group(ui, language.text("会话"), |ui| {
                                    ui.label(language.text("回复期间的发送方式"));
                                    let help = language.text(
                                        "加入队列会在当前回复结束后发送。引导会话会把消息送入正在回复的当前轮。",
                                    );
                                    ui.label(RichText::new(help).color(palette.muted));
                                    ui.add_space(8.);
                                    egui::ComboBox::from_id_salt("send-mode")
                                        .width(160.)
                                        .selected_text(language.text(self.send_mode.label()))
                                        .show_ui(ui, |ui| {
                                            for mode in [SendMode::Queue, SendMode::Steer] {
                                                ui.selectable_value(
                                                    &mut self.send_mode,
                                                    mode,
                                                    language.text(mode.label()),
                                                );
                                            }
                                        });
                                });
                            }
                            SettingsTab::Cli => {
                                settings_group(ui, language.text("CLI 路径"), |ui| {
                                    save_paths = cli_path_settings(ui, page, language);
                                });
                                ui.add_space(12.);
                                settings_group(ui, language.text("网络"), |ui| {
                                    save = proxy_settings(ui, page, language);
                                });
                            }
                            SettingsTab::Updates => {
                                settings_group(ui, language.text("软件更新"), |ui| {
                                    restart = self.updates.show(ui, language);
                                });
                            }
                        });
                        ui.add_space(24.);
                    });
            });
        if self.language != language {
            ctx.request_repaint();
        }
        if self.appearance != appearance {
            self.appearance.apply(ctx);
        }
        if back {
            self.settings_page = None;
        } else if restart {
            self.restart_for_update(ctx);
        } else if save_paths {
            self.save_cli_path_settings(ctx);
        } else if save {
            self.save_proxy_settings(ctx);
        }
    }

    fn questions(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        let language = self.language;
        let Some(snapshot) = self.view.snapshot.as_ref().filter(|_| !self.view.stopping) else {
            self.view.question_form = None;
            return;
        };
        let Some(request) = &snapshot.question else {
            self.view.question_form = None;
            return;
        };
        if self
            .view
            .question_form
            .as_ref()
            .is_none_or(|form| form.id != request.id)
        {
            self.view.question_form = Some(QuestionForm {
                id: request.id.clone(),
                open: true,
                drafts: request
                    .questions
                    .iter()
                    .map(|question| QuestionDraft {
                        use_custom: question.options.is_empty(),
                        ..Default::default()
                    })
                    .collect(),
                error: None,
            });
        }
        let form = self.view.question_form.as_mut().unwrap();
        if !form.open {
            return;
        }
        let modal_id = egui::Id::new(("questions", &request.id));
        let open_t = ctx.animate_bool_with_time(modal_id.with("backdrop"), true, 0.18);
        let modal = egui::Modal::new(modal_id)
            .frame(popup(palette, Margin::same(20)))
            .backdrop_color(palette.backdrop.gamma_multiply(open_t))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 64.).clamp(190., 500.));
                ui.label(
                    RichText::new(language.text("需要你回答"))
                        .size(20.)
                        .strong(),
                );
                ui.add_space(6.);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height((ctx.content_rect().height() * 0.60).max(80.))
                    .show(ui, |ui| {
                        for (index, (question, draft)) in
                            request.questions.iter().zip(&mut form.drafts).enumerate()
                        {
                            if index > 0 {
                                ui.add_space(7.);
                                ui.separator();
                            }
                            ui.push_id(index, |ui| draft.show(ui, question));
                        }
                    });
                if let Some(error) = &form.error {
                    ui.label(RichText::new(language.text(error)).color(palette.warn));
                }
                ui.add_space(8.);
                let answers: QuestionAnswers = request
                    .questions
                    .iter()
                    .zip(&form.drafts)
                    .map(|(question, draft)| (question.id.clone(), draft.answers(question)))
                    .collect();
                let ready = request
                    .questions
                    .iter()
                    .all(|question| question.accepts(&answers[&question.id]));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let fill = if ready { palette.accent } else { palette.soft };
                    let text = if ready {
                        palette.on_accent
                    } else {
                        palette.muted
                    };
                    let submit = ui
                        .add_enabled(
                            ready && !self.run_command_pending,
                            Button::new(RichText::new(language.text("提交答案")).color(text))
                                .fill(fill)
                                .min_size(vec2(90., 32.)),
                        )
                        .clicked();
                    let later = ui.button(language.text("稍后回答")).clicked();
                    (submit.then_some(answers), later)
                })
                .inner
            });
        if let (Some(answers), _) = modal.inner {
            self.run_command(
                ctx,
                snapshot.run.id.clone(),
                RunCommand::Answer {
                    question_id: request.id.clone(),
                    answers,
                },
            );
        } else if modal.inner.1 || (!modal.backdrop_response.clicked() && modal.should_close()) {
            form.open = false;
        }
    }

    fn paint_quota_dialog(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        let modal = show_quota_modal(ctx, |ui| {
            let language = Language::in_ui(ui);
            let provider = self.quota_provider;
            let (spinning, can_refresh, refresh_label) = if self.showing_account_quota {
                let pending = self.quota_pending.contains(&provider);
                let resetting = provider == Provider::Codex && self.codex_quota_reset.pending;
                (pending, !pending && !resetting, "刷新额度")
            } else {
                (
                    self.token_usage_pending,
                    !self.token_usage_pending,
                    "刷新用量",
                )
            };
            let mut close = false;
            let mut refresh = false;
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 34.),
                Layout::left_to_right(Align::Center),
                |ui| {
                    let (icon, _) = ui.allocate_exact_size(vec2(32., 32.), egui::Sense::hover());
                    ui.painter().rect_filled(icon, 9., palette.tint);
                    Icon::Usage.paint(ui.painter(), icon.center(), palette.accent);
                    ui.add_space(3.);
                    ui.label(RichText::new(language.text("用量")).size(20.).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close = Icon::Close
                            .button(ui, language.text("关闭"), palette.muted)
                            .clicked();
                        ui.add_enabled_ui(can_refresh, |ui| {
                            refresh = Icon::refresh_button(
                                ui,
                                language.text(refresh_label),
                                palette.muted,
                                spinning && ui.is_visible(),
                            )
                            .clicked();
                        });
                    });
                },
            );
            ui.add_space(6.);
            if usage_section_tabs(ui, &mut self.showing_account_quota) {
                refresh = true;
            }
            ui.add_space(10.);
            if self.showing_account_quota {
                if refresh {
                    self.load_quota(ctx, provider);
                }
                self.quota_content(ui, ctx);
            } else {
                if refresh {
                    self.load_token_usage(ctx);
                }
                self.token_usage_content(ui);
            }
            close
        });
        if modal.inner || modal.should_close() {
            self.quota_dialog = false;
            let reset = &mut self.codex_quota_reset;
            if !reset.pending && reset.error.is_none() {
                reset.request = None;
            }
            ctx.request_repaint();
        }
    }

    fn token_usage_content(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let periods = UsagePeriod::ALL.map(|period| (period, language.text(period.label())));
        if let Some(period) = segmented_bar(ui, self.token_period, &periods) {
            self.token_period = period;
        }
        ui.add_space(10.);
        if let Some(error) = &self.token_usage_error {
            ui.label(RichText::new(language.text(error)).color(palette.warn));
            if self.token_usage.is_none() {
                return;
            }
            ui.add_space(6.);
        }
        let Some(window) = self.token_usage.as_ref().and_then(|report| {
            report
                .windows
                .iter()
                .find(|window| window.period == self.token_period)
        }) else {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.).color(palette.accent));
                ui.label(RichText::new(language.text("正在读取 Token 用量…")).color(palette.muted));
            });
            return;
        };
        render_token_usage_window(ui, window);
        ui.add_space(8.);
        ui.label(
            RichText::new(language.text("统计所连接电脑上的 CLI 历史。输入含缓存读取和写入。"))
                .color(palette.muted)
                .size(11.5),
        );
    }

    fn quota_content(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let provider = self.quota_provider;
        let providers = Provider::ALL.map(|choice| (choice, choice.label()));
        let next_provider = segmented_bar(ui, provider, &providers);
        ui.add_space(10.);
        let mut reset_credits = None;
        if self.quota_pending.contains(&provider) {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.).color(palette.accent));
                ui.label(RichText::new(language.text("正在读取额度…")).color(palette.muted));
            });
        } else if let Some(result) = self.quotas.get(&provider) {
            match result {
                Ok(quota) => {
                    render_quota(ui, quota);
                    reset_credits = quota.reset_credits;
                }
                Err(error) => {
                    ui.label(RichText::new(language.text(error)).color(palette.warn));
                }
            }
        }
        if provider == Provider::Codex {
            self.quota_reset_controls(ui, ctx, reset_credits);
        }
        if let Some(provider) = next_provider {
            self.quota_provider = provider;
            self.load_quota(ctx, provider);
            ctx.request_repaint();
        }
    }

    fn quota_reset_controls(&mut self, ui: &mut Ui, ctx: &egui::Context, credits: Option<u64>) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let reset = &mut self.codex_quota_reset;
        if credits.is_none() && reset.request.is_none() && reset.message.is_none() {
            return;
        }
        let confirming = reset.request.is_some();
        let mut submit = false;
        surface(palette, palette.paper, Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(count) = credits {
                ui.horizontal(|ui| {
                    ui.label(
                        language.format("可用重置次数 {count}", &[("count", &count.to_string())]),
                    );
                    if !confirming {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(count > 0, Button::new(language.text("重置额度")))
                                .clicked()
                            {
                                reset.request = Some(uuid::Uuid::new_v4());
                                reset.message = None;
                                ctx.request_repaint();
                            }
                        });
                    }
                });
            }
            if confirming {
                ui.label(
                    RichText::new(
                        language.text("使用 1 次重置机会，立即重置符合条件的 Codex 额度。"),
                    )
                    .color(palette.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if reset.pending {
                        language.text("正在重置…")
                    } else if reset.error.is_some() {
                        language.text("重试重置")
                    } else {
                        language.text("确认重置")
                    };
                    submit = ui.add_enabled(!reset.pending, Button::new(label)).clicked();
                    if !reset.pending
                        && reset.error.is_none()
                        && ui.button(language.text("取消")).clicked()
                    {
                        reset.request = None;
                        ctx.request_repaint();
                    }
                });
            }
            if let Some(error) = &reset.error {
                ui.label(RichText::new(language.text(error)).color(palette.warn));
            }
            if let Some(message) = reset.message {
                ui.label(RichText::new(language.text(message)).color(palette.muted));
            }
        });
        if submit {
            self.submit_quota_reset(ctx);
        }
    }

    fn devices(&mut self, ctx: &egui::Context) {
        let palette = Palette::from_context(ctx);
        let language = self.language;
        let backdrop_id = egui::Id::new("device connection backdrop");
        if !self.pair_dialog {
            ctx.animate_bool_with_time(backdrop_id, false, 0.01);
            return;
        }
        let open_t = ctx.animate_bool_with_time(backdrop_id, true, 0.18);
        let remote = self.gateway.remote().status();
        let device_url = remote.device_url.as_deref().filter(|_| remote.connected);
        let code = self.gateway.pairing_code();
        if let Some(address) = device_url {
            let link = pairing_link(address, &code);
            if self.pairing_qr.as_ref().is_none_or(|qr| qr.link != link) {
                self.pairing_qr = Some(PairingQr::new(ctx, link));
            }
        } else {
            self.pairing_qr = None;
        }
        let local_link = pairing_link(self.gateway.local_url(), &code);
        let mut tab = self.device_tab;
        let modal = show_device_modal(
            ctx,
            &mut tab,
            remote.connected,
            &remote.detail,
            open_t,
            |ui, tab| match (tab, device_url) {
                (DeviceTab::Connect, _) => self.connect_to_device(ui),
                (DeviceTab::PairThisComputer, Some(address)) => {
                    pairing_card(
                        ui,
                        self.pairing_qr.as_ref().and_then(|qr| qr.texture.as_ref()),
                        address,
                        &code,
                    );
                    ui.add_space(4.);
                    egui::CollapsingHeader::new(language.text("中继设置"))
                        .id_salt("relay server")
                        .show(ui, |ui| self.relay_fields(ui));
                }
                (DeviceTab::PairThisComputer, None) => {
                    surface(palette, palette.canvas, Margin::same(14)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(language.text("让其他设备连接本机"))
                                .size(16.)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(language.text("连接中继服务器后即可显示配对二维码。"))
                                .color(palette.muted)
                                .size(13.),
                        );
                        ui.label(
                            RichText::new(language.text(&remote.detail))
                                .color(palette.warn)
                                .size(13.),
                        );
                        ui.add_space(4.);
                        self.relay_fields(ui);
                    });
                }
            },
            |ui| {
                if ui.button(language.text("打开本机网页")).clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(local_link));
                }
                if let Some(url) = &remote.url
                    && ui.button(language.text("打开中继网页")).clicked()
                {
                    ctx.open_url(egui::OpenUrl::new_tab(url.clone()));
                }
            },
        );
        self.device_tab = tab;
        if modal.inner || modal.should_close() {
            self.pair_dialog = false;
            self.pairing_qr = None;
        }
    }

    fn connect_to_device(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        let pending = self.target_device_pending;
        let can_switch = self.can_switch_workspace();
        surface(palette, palette.canvas, Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(device) = &self.remote {
                let return_to_local = ui
                    .horizontal(|ui| {
                        ui.label(RichText::new(language.text("当前设备")).color(palette.muted));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let clicked = ui
                                .add_enabled(can_switch, Button::new(language.text("返回本机")))
                                .clicked();
                            ui.add(Label::new(RichText::new(device.name()).strong()).truncate())
                                .on_hover_text(device.name());
                            clicked
                        })
                        .inner
                    })
                    .inner;
                ui.add(
                    Label::new(
                        RichText::new(device.address())
                            .monospace()
                            .size(12.)
                            .color(palette.muted),
                    )
                    .wrap(),
                );
                if return_to_local {
                    self.return_to_local(ui.ctx());
                }
                return;
            }
            if let Some((device, _)) = &self.recent_device {
                let label = language.format("重新连接 {name}", &[("name", device.name())]);
                if ui.add_enabled(can_switch, Button::new(label)).clicked() {
                    self.reopen_device(ui.ctx());
                    return;
                }
                ui.add_space(5.);
                ui.separator();
            }
            if remote_pairing_form(
                ui,
                &mut self.target_device_url,
                &mut self.target_device_code,
                &mut self.target_device_error,
                pending,
                can_switch,
            ) {
                self.connect_device(ui.ctx());
            }
        });
    }

    fn relay_fields(&mut self, ui: &mut Ui) {
        let palette = Palette::of(ui);
        let language = Language::in_ui(ui);
        ui.label(
            RichText::new(language.text("服务器地址"))
                .color(palette.muted)
                .size(12.5),
        );
        input::text_edit(
            ui,
            TextEdit::singleline(&mut self.relay_url)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(9, 7))
                .hint_text("https://naro.example.com"),
            false,
        );
        ui.label(
            RichText::new(language.text("连接密钥"))
                .color(palette.muted)
                .size(12.5),
        );
        input::text_edit(
            ui,
            TextEdit::singleline(&mut self.relay_token)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(9, 7)),
            true,
        );
        let connect = if cfg!(windows) {
            language.text("保存并连接")
        } else {
            language.text("连接")
        };
        if solid(ui, palette.accent, palette.accent_hover, |ui| {
            ui.add(
                Button::new(RichText::new(connect).color(palette.on_accent))
                    .min_size(vec2(0., 33.)),
            )
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
            ui.label(RichText::new(language.text(note)).color(palette.warn));
        }
    }
}

fn show_device_modal(
    ctx: &egui::Context,
    tab: &mut DeviceTab,
    online: bool,
    detail: &str,
    opacity: f32,
    body: impl FnOnce(&mut Ui, DeviceTab),
    footer: impl FnOnce(&mut Ui),
) -> egui::ModalResponse<bool> {
    let palette = Palette::from_context(ctx);
    let screen = ctx.content_rect();
    egui::Modal::new(egui::Id::new("device connection"))
        .frame(popup(palette, Margin::same(20)).corner_radius(18))
        .backdrop_color(palette.backdrop.gamma_multiply(opacity))
        .show(ctx, |ui| {
            let language = Language::in_ui(ui);
            ui.set_width((screen.width() - 64.).clamp(1., 520.));
            ui.spacing_mut().item_spacing.y = 5.;
            let close = ui
                .allocate_ui_with_layout(
                    vec2(ui.available_width(), 36.),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        let (icon, _) =
                            ui.allocate_exact_size(vec2(34., 34.), egui::Sense::hover());
                        ui.painter().rect_filled(icon, 9., palette.tint);
                        Icon::Devices.paint(ui.painter(), icon.center(), palette.accent);
                        ui.add_space(3.);
                        ui.label(RichText::new(language.text("设备连接")).size(22.).strong());
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 36.),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                let close = Icon::Close
                                    .button(ui, language.text("关闭"), palette.muted)
                                    .clicked();
                                if *tab != DeviceTab::PairThisComputer {
                                    return close;
                                }
                                ui.add_space(4.);
                                let (label, fill, ink) = if online {
                                    ("本机在线", palette.online_bg, palette.online)
                                } else {
                                    ("本机未上线", palette.soft, palette.muted)
                                };
                                let label = language.text(label);
                                let galley = ui.fonts_mut(|fonts| {
                                    fonts.layout_no_wrap(
                                        label.to_owned(),
                                        FontId::proportional(12.),
                                        ink,
                                    )
                                });
                                let (chip, response) = ui.allocate_exact_size(
                                    galley.size() + vec2(32., 8.),
                                    egui::Sense::hover(),
                                );
                                let painter = ui.painter();
                                painter.rect_filled(chip, chip.height() / 2., fill);
                                painter.circle_filled(
                                    egui::pos2(chip.left() + 13., chip.center().y),
                                    3.,
                                    ink,
                                );
                                painter.galley(chip.min + vec2(22., 4.), galley, ink);
                                response.widget_info(|| {
                                    WidgetInfo::labeled(WidgetType::Label, true, label)
                                });
                                response.on_hover_text(language.text(detail));
                                close
                            },
                        )
                        .inner
                    },
                )
                .inner;
            ui.add_space(6.);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.;
                for choice in [DeviceTab::Connect, DeviceTab::PairThisComputer] {
                    let selected = *tab == choice;
                    if ui
                        .add_sized(
                            vec2(width, 32.),
                            Button::new(RichText::new(language.text(choice.label())).strong())
                                .selected(selected)
                                .frame_when_inactive(selected),
                        )
                        .clicked()
                    {
                        *tab = choice;
                    }
                }
            });
            ui.add_space(8.);
            egui::ScrollArea::vertical()
                .id_salt(*tab)
                .auto_shrink([false, true])
                .max_height((screen.height() - 194.).max(1.))
                .show(ui, |ui| body(ui, *tab));
            if *tab == DeviceTab::PairThisComputer {
                ui.add_space(4.);
                ui.separator();
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 28.),
                    Layout::right_to_left(Align::Center),
                    footer,
                );
            }
            close
        })
}

fn remote_pairing_form(
    ui: &mut Ui,
    address: &mut String,
    code: &mut String,
    error: &mut Option<String>,
    pending: bool,
    can_switch: bool,
) -> bool {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    ui.add(
        Label::new(
            RichText::new(language.text("在目标电脑打开「设备连接 → 本机配对 → 手动配对」。"))
                .size(12.5)
                .color(palette.muted),
        )
        .wrap(),
    );
    ui.add_space(4.);
    let address_label = ui.label(RichText::new(language.text("设备地址")).color(palette.muted));
    let address_input = ui
        .add_enabled_ui(!pending, |ui| {
            input::text_edit(
                ui,
                TextEdit::singleline(address)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(9, 7))
                    .hint_text(RichText::new("https://…").color(palette.muted)),
                false,
            )
            .labelled_by(address_label.id)
        })
        .inner;
    let (code_input, clicked, can_pair) = ui
        .allocate_ui_with_layout(
            vec2(ui.available_width(), 36.),
            Layout::left_to_right(Align::Center),
            |ui| {
                let code_label =
                    ui.label(RichText::new(language.text("配对码")).color(palette.muted));
                let code_input = ui
                    .add_enabled_ui(!pending, |ui| {
                        input::text_edit(
                            ui,
                            TextEdit::singleline(code)
                                .desired_width(118.)
                                .margin(Margin::symmetric(9, 7))
                                .hint_text(
                                    RichText::new(language.text("8 位数字")).color(palette.muted),
                                ),
                            false,
                        )
                        .labelled_by(code_label.id)
                    })
                    .inner;
                let can_pair = can_switch
                    && !address.trim().is_empty()
                    && (!code.trim().is_empty() || address.contains("#pair="));
                let label = if pending {
                    "正在配对…"
                } else {
                    "配对并连接"
                };
                let clicked = ui
                    .allocate_ui_with_layout(
                        vec2(ui.available_width(), 36.),
                        Layout::right_to_left(Align::Center),
                        |ui| action_dialog_primary(ui, language.text(label), can_pair),
                    )
                    .inner;
                (code_input, clicked, can_pair)
            },
        )
        .inner;
    if address_input.changed() || code_input.changed() {
        *error = None;
    }
    if let Some(error) = error {
        ui.add(Label::new(RichText::new(language.text(error)).color(palette.warn)).wrap());
    }
    let enter = (address_input.lost_focus() || code_input.lost_focus())
        && ui.input(|input| input.key_pressed(egui::Key::Enter));
    can_pair && (clicked || enter)
}

fn pairing_card(ui: &mut Ui, qr: Option<&egui::TextureHandle>, address: &str, code: &str) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    surface(palette, palette.tint, Margin::same(12))
        .stroke(Stroke::new(1., palette.tint_line))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let height = if qr.is_some() { QR_DISPLAY_SIZE } else { 52. };
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), height),
                Layout::left_to_right(Align::Center),
                |ui| {
                    if let Some(texture) = qr {
                        PairingQr::paint(texture, ui);
                        ui.add_space(12.);
                    }
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(language.text("手机扫码连接"))
                                .size(17.)
                                .strong(),
                        );
                        let hint = if qr.is_some() {
                            "用手机相机扫描二维码，打开链接即可连接本机。"
                        } else {
                            "无法生成二维码，请展开手动配对。"
                        };
                        ui.add(
                            Label::new(
                                RichText::new(language.text(hint))
                                    .size(13.)
                                    .color(palette.muted),
                            )
                            .wrap(),
                        );
                    });
                },
            );
        });
    egui::CollapsingHeader::new(language.text("手动配对"))
        .id_salt("manual pairing")
        .show(ui, |ui| {
            surface(palette, palette.canvas, Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(language.text("配对码"))
                            .size(13.)
                            .color(palette.muted),
                    );
                    ui.label(
                        RichText::new(code)
                            .monospace()
                            .size(19.)
                            .color(palette.accent),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(language.text("复制设备地址")).clicked() {
                            ui.ctx().copy_text(address.to_owned());
                        }
                    });
                });
                ui.add(
                    Label::new(
                        RichText::new(address)
                            .monospace()
                            .size(12.)
                            .color(palette.muted),
                    )
                    .wrap(),
                );
            });
        });
}

fn show_quota_modal(
    ctx: &egui::Context,
    mut contents: impl FnMut(&mut Ui) -> bool,
) -> egui::ModalResponse<bool> {
    let palette = Palette::from_context(ctx);
    let id = egui::Id::new("quota dialog");
    let screen = ctx.content_rect();
    let available = screen.shrink(16.).size().max(vec2(1., 1.));
    let frame = popup(palette, Margin::same(18)).corner_radius(18);
    let width = available.x.clamp(360., 640.);
    let layout = Rect::from_min_size(egui::Pos2::ZERO, vec2(width, f32::INFINITY));
    let mut draw = |ui: &mut Ui| {
        frame
            .show(ui, |ui| {
                ui.set_width(width - frame.total_margin().sum().x);
                contents(ui)
            })
            .inner
    };

    let mut measurement = Ui::new(
        ctx.clone(),
        id.with("measurement"),
        egui::UiBuilder::new().max_rect(layout).invisible(),
    );
    draw(&mut measurement);
    let size = measurement.min_size();
    // Fit the measured content while keeping the backdrop and its hit area at full size.
    let scale = (available / size).min_elem().min(1.);
    let bounds = Rect::from_center_size(screen.center(), size * scale);
    let transform = egui::emath::TSTransform::new(bounds.min.to_vec2(), scale);

    egui::Modal::new(id)
        .area(
            egui::Modal::default_area(id)
                .fixed_pos(bounds.min)
                .default_size(bounds.size())
                .constrain(false)
                .fade_in(false),
        )
        .frame(egui::Frame::NONE)
        .backdrop_color(palette.backdrop)
        .show(ctx, |ui| {
            ui.set_min_size(bounds.size());
            let layer = egui::LayerId::new(ui.layer_id().order, id.with("contents"));
            ctx.set_sublayer(ui.layer_id(), layer);
            ctx.set_transform_layer(layer, transform);
            let mut content = ui.new_child(
                egui::UiBuilder::new()
                    .layer_id(layer)
                    .max_rect(layout)
                    .sense(egui::Sense::click_and_drag()),
            );
            content.set_clip_rect(transform.inverse() * screen);
            draw(&mut content)
        })
}

fn usage_section_tabs(ui: &mut Ui, showing_account_quota: &mut bool) -> bool {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let mut changed = false;
    let width = ui.available_width();
    let tab_height = 32.;
    let full_left = ui.cursor().left();
    let baseline_y = ui.cursor().top() + tab_height - 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(full_left, baseline_y),
            egui::pos2(full_left + width, baseline_y),
        ],
        Stroke::new(1., palette.line),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 20.;
        for (account, label) in [(false, "Token 用量"), (true, "账号额度")] {
            let selected = *showing_account_quota == account;
            let text = language.text(label);
            let font = FontId::proportional(14.);
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(text.to_owned(), font.clone(), palette.ink)
            });
            let size = vec2(galley.size().x + 4., tab_height);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, text)
            });
            let color = if selected || response.hovered() {
                palette.ink
            } else {
                palette.muted
            };
            let galley = ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, color));
            let text_pos = egui::pos2(
                rect.center().x - galley.size().x / 2.,
                rect.top() + (rect.height() - galley.size().y - 4.) / 2.,
            );
            ui.painter().galley(text_pos, galley, color);
            if selected {
                let bar = Rect::from_min_max(
                    egui::pos2(rect.left(), rect.bottom() - 2.5),
                    egui::pos2(rect.right(), rect.bottom()),
                );
                ui.painter().rect_filled(bar, 1.25, palette.accent);
            }
            if response.clicked() && !selected {
                *showing_account_quota = account;
                changed = true;
            }
        }
    });
    changed
}

fn segmented_bar<T: Copy + PartialEq>(ui: &mut Ui, current: T, items: &[(T, &str)]) -> Option<T> {
    let palette = Palette::of(ui);
    let mut selected_next = None;
    egui::Frame::new()
        .fill(palette.soft)
        .corner_radius(10)
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.;
                for &(value, label) in items {
                    let active = current == value;
                    let font = FontId::proportional(12.5);
                    let color = if active { palette.ink } else { palette.muted };
                    let galley = ui.fonts_mut(|fonts| {
                        fonts.layout_no_wrap(label.to_owned(), font.clone(), color)
                    });
                    let size = vec2((galley.size().x + 22.).max(76.), 26.);
                    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                    response.widget_info(|| {
                        WidgetInfo::selected(WidgetType::SelectableLabel, true, active, label)
                    });
                    let painter = ui.painter();
                    if active {
                        painter.rect(
                            rect,
                            7.,
                            palette.paper,
                            Stroke::new(1., palette.line),
                            StrokeKind::Inside,
                        );
                    } else if response.hovered() {
                        painter.rect_filled(rect, 7., palette.paper.gamma_multiply(0.55));
                    }
                    let text_color = if active || response.hovered() {
                        palette.ink
                    } else {
                        palette.muted
                    };
                    let galley = ui.fonts_mut(|fonts| {
                        fonts.layout_no_wrap(label.to_owned(), font, text_color)
                    });
                    let pos = rect.center() - galley.size() / 2.;
                    painter.galley(pos, galley, text_color);
                    if response.clicked() && !active {
                        selected_next = Some(value);
                    }
                }
            });
        });
    selected_next
}

fn provider_accent(ui: &Ui, palette: Palette, provider: Provider) -> Color32 {
    match provider {
        Provider::Codex => palette.accent,
        Provider::Claude => {
            if ui.visuals().dark_mode {
                Color32::from_rgb(228, 155, 114)
            } else {
                Color32::from_rgb(199, 111, 69)
            }
        }
        Provider::Antigravity => palette.online,
    }
}

fn format_share(part: u64, total: u64) -> Option<String> {
    if total == 0 || part == 0 {
        return None;
    }
    let tenths = ((u128::from(part) * 1000) + u128::from(total) / 2) / u128::from(total);
    if tenths == 0 {
        Some("<0.1%".into())
    } else if tenths % 10 == 0 {
        Some(format!("{}%", tenths / 10))
    } else {
        Some(format!("{}.{}%", tenths / 10, tenths % 10))
    }
}

fn render_quota(ui: &mut Ui, quota: &Quota) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    if quota.limits.is_empty() {
        ui.label(RichText::new(language.text("暂未返回用量窗口。")).color(palette.muted));
    }
    let now = crate::engine::unix_time(std::time::SystemTime::now());
    for (group_index, limit) in quota.limits.iter().enumerate() {
        if group_index > 0 {
            ui.add_space(4.);
        }
        surface(palette, palette.paper, Margin::symmetric(16, 12))
            .corner_radius(14)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.;
                ui.label(
                    RichText::new(language.text(&limit.name))
                        .size(13.)
                        .color(palette.muted)
                        .strong(),
                );
                for (row_index, window) in limit.windows.iter().enumerate() {
                    ui.push_id((group_index, row_index), |ui| {
                        render_quota_window(ui, window, now)
                    });
                }
            });
    }
    let balance = if quota.unlimited_credits {
        Some(language.text("不限"))
    } else {
        quota.credit_balance.as_deref()
    };
    if let Some(balance) = balance {
        ui.add_space(4.);
        surface(palette, palette.canvas, Margin::symmetric(16, 10))
            .corner_radius(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(language.format("积分余额 {balance}", &[("balance", balance)]))
                        .size(13.)
                        .color(palette.muted),
                );
            });
    }
}

fn render_token_usage_window(ui: &mut Ui, window: &engine::usage::UsageWindow) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let grand_total = window.total.map_or(0, TokenCounts::total);
    let any_partial = window.providers.iter().any(|provider| provider.partial);

    // Tier 1: Hero summary card
    egui::Frame::new()
        .fill(palette.notice)
        .stroke(Stroke::new(1., palette.tint_line))
        .corner_radius(14)
        .inner_margin(Margin::symmetric(16, 13))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Sides::new().shrink_left().show(
                ui,
                |ui| {
                    ui.label(
                        RichText::new(language.text("总消耗"))
                            .size(12.5)
                            .color(palette.muted)
                            .strong(),
                    );
                },
                |ui| {
                    if any_partial {
                        ui.label(
                            RichText::new(language.text("部分 Token 记录暂不可用。"))
                                .size(11.5)
                                .color(palette.muted),
                        );
                    }
                },
            );
            if let Some(total) = window.total {
                let count = total.total();
                let hover = format!("{} Tokens", group_tokens(count));
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    &format_token_count(count),
                    0.,
                    egui::TextFormat {
                        font_id: FontId::proportional(28.),
                        color: palette.ink,
                        ..Default::default()
                    },
                );
                job.append(
                    " Tokens",
                    4.,
                    egui::TextFormat {
                        font_id: FontId::proportional(13.),
                        color: palette.muted,
                        valign: Align::BOTTOM,
                        ..Default::default()
                    },
                );
                ui.label(job).on_hover_text(&hover);
                if grand_total > 0 {
                    ui.add_space(4.);
                    render_stacked_share_bar(ui, &window.providers, grand_total);
                }
                ui.add_space(8.);
                let (divider, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.), egui::Sense::hover());
                ui.painter().rect_filled(divider, 0., palette.tint_line);
                ui.add_space(8.);
                render_token_metric_columns(ui, total, true);
            } else {
                ui.add_space(2.);
                ui.label(
                    RichText::new(language.text("暂无 Token 记录。"))
                        .size(13.)
                        .color(palette.muted),
                );
            }
        });

    ui.add_space(12.);
    ui.label(
        RichText::new(language.text("各引擎明细"))
            .size(12.)
            .color(palette.muted)
            .strong(),
    );
    ui.add_space(4.);

    // Tier 2: Unified provider breakdown card
    surface(palette, palette.paper, Margin::symmetric(16, 4))
        .corner_radius(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (index, provider) in window.providers.iter().enumerate() {
                if index > 0 {
                    let (divider, _) = ui
                        .allocate_exact_size(vec2(ui.available_width(), 1.), egui::Sense::hover());
                    ui.painter().rect_filled(divider, 0., palette.line);
                }
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(0, 9))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        render_provider_usage_row(ui, provider, grand_total);
                    });
            }
        });
}

fn render_stacked_share_bar(
    ui: &mut Ui,
    providers: &[engine::usage::ProviderUsage],
    grand_total: u64,
) {
    let palette = Palette::of(ui);
    let segments: Vec<_> = providers
        .iter()
        .filter_map(|item| {
            let count = item.counts?.total();
            (count > 0).then_some((item.provider, count))
        })
        .collect();
    if segments.is_empty() || grand_total == 0 {
        return;
    }
    let width = ui.available_width();
    let height = 6.;
    let (track, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    ui.painter().rect_filled(track, height / 2., palette.soft);
    let gap = 2.;
    let total_gap = gap * (segments.len().saturating_sub(1) as f32);
    let usable = (width - total_gap).max(1.);
    let min_seg = 4.;
    let raw: Vec<f32> = segments
        .iter()
        .map(|(_, count)| (usable * (*count as f32 / grand_total as f32)).max(min_seg))
        .collect();
    let raw_sum: f32 = raw.iter().sum();
    let scale = if raw_sum > usable {
        usable / raw_sum
    } else {
        1.
    };
    let mut x = track.left();
    for (index, ((provider, count), seg_width)) in segments.iter().zip(raw).enumerate() {
        let w = (seg_width * scale).max(2.);
        let right = if index + 1 == segments.len() {
            track.right()
        } else {
            (x + w).min(track.right())
        };
        let seg_rect = Rect::from_min_max(
            egui::pos2(x, track.top()),
            egui::pos2(right, track.bottom()),
        );
        let color = provider_accent(ui, palette, *provider);
        ui.painter().rect_filled(seg_rect, height / 2., color);
        let share = format_share(*count, grand_total).unwrap_or_default();
        ui.interact(
            seg_rect,
            ui.id().with(("share_seg", index)),
            egui::Sense::hover(),
        )
        .on_hover_text(format!(
            "{} · {} Tokens ({share})",
            provider.label(),
            group_tokens(*count)
        ));
        x = right + gap;
    }
}

fn render_token_metric_columns(ui: &mut Ui, counts: TokenCounts, hero: bool) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let fields = [
        ("输入 Token", counts.input_tokens),
        ("输出 Token", counts.output_tokens),
        ("缓存读取", counts.cache_read_tokens),
        ("缓存写入", counts.cache_write_tokens),
    ];
    let label_size = if hero { 11. } else { 10.5 };
    let value_size = if hero { 14. } else { 12.5 };
    ui.columns(fields.len(), |columns| {
        for (column, (label, value)) in columns.iter_mut().zip(fields) {
            column.spacing_mut().item_spacing.y = 1.5;
            column.label(
                RichText::new(language.text(label))
                    .size(label_size)
                    .color(palette.muted),
            );
            let value_color = if value == 0 {
                palette.muted
            } else {
                palette.ink
            };
            let mut text = RichText::new(format_token_count(value))
                .size(value_size)
                .color(value_color);
            if hero && value > 0 {
                text = text.strong();
            }
            column
                .label(text)
                .on_hover_text(format!("{} Tokens", group_tokens(value)));
        }
    });
}

fn render_provider_usage_row(
    ui: &mut Ui,
    provider: &engine::usage::ProviderUsage,
    grand_total: u64,
) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let accent = provider_accent(ui, palette, provider.provider);
    let count = provider.counts.map(TokenCounts::total);
    let share = count.and_then(|value| format_share(value, grand_total));

    egui::Sides::new().height(22.).shrink_left().show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = 7.;
            let (dot, _) = ui.allocate_exact_size(vec2(8., 8.), egui::Sense::hover());
            let dot_color = if count.is_some_and(|value| value > 0) {
                accent
            } else {
                palette.line
            };
            ui.painter().circle_filled(dot.center(), 4., dot_color);
            ui.label(RichText::new(provider.provider.label()).size(13.5).strong());
        },
        |ui| {
            if let Some(value) = count {
                let hover = format!("{} Tokens", group_tokens(value));
                ui.spacing_mut().item_spacing.x = 5.;
                ui.label(RichText::new("Tokens").size(11.5).color(palette.muted))
                    .on_hover_text(&hover);
                let value_color = if value == 0 {
                    palette.muted
                } else {
                    palette.ink
                };
                ui.label(
                    RichText::new(format_token_count(value))
                        .size(14.)
                        .color(value_color)
                        .strong(),
                )
                .on_hover_text(&hover);
                if let Some(share) = &share {
                    ui.add_space(2.);
                    let galley = ui.fonts_mut(|fonts| {
                        fonts.layout_no_wrap(
                            share.clone(),
                            FontId::proportional(11.),
                            palette.muted,
                        )
                    });
                    let (chip, _) =
                        ui.allocate_exact_size(galley.size() + vec2(10., 4.), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(chip, chip.height() / 2., palette.soft);
                    ui.painter()
                        .galley(chip.center() - galley.size() / 2., galley, palette.muted);
                }
            } else {
                ui.label(
                    RichText::new(language.text("暂无 Token 记录。"))
                        .size(12.)
                        .color(palette.muted),
                );
            }
        },
    );

    if let Some(counts) = provider.counts {
        let value = counts.total();
        if grand_total > 0 {
            ui.add_space(3.);
            let width = ui.available_width();
            let height = 3.;
            let (track, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
            ui.painter().rect_filled(track, height / 2., palette.soft);
            if value > 0 {
                let fill_w = (width * (value as f32 / grand_total as f32)).clamp(3., width);
                let fill = Rect::from_min_size(track.min, vec2(fill_w, height));
                ui.painter().rect_filled(fill, height / 2., accent);
            }
        }
        ui.add_space(5.);
        render_token_metric_columns(ui, counts, false);
    }
    if provider.partial {
        ui.add_space(4.);
        ui.label(
            RichText::new(language.text("部分 Token 记录暂不可用。"))
                .size(11.)
                .color(palette.muted),
        );
    }
}

fn format_token_count(count: u64) -> String {
    if count < 1_000_000 {
        return group_tokens(count);
    }
    let (step, unit) = if count >= 999_995_000 {
        (10_000_000, "B")
    } else {
        (10_000, "M")
    };
    let hundredths = count / step + u64::from(count % step >= step / 2);
    let whole = group_tokens(hundredths / 100);
    match hundredths % 100 {
        0 => format!("{whole}{unit}"),
        fraction => format!(
            "{whole}.{}{unit}",
            format!("{fraction:02}").trim_end_matches('0')
        ),
    }
}

fn group_tokens(count: u64) -> String {
    let digits = count.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(char::from(digit));
    }
    formatted
}

fn render_quota_window(ui: &mut Ui, window: &engine::QuotaWindow, now: i64) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let amount = window
        .remaining_percent
        .map(|percent| format!("{percent}%"))
        .or_else(|| window.remaining_amount.map(|amount| format!("{amount:.2}")));
    let reset = window
        .resets_at
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0));
    egui::Sides::new()
        .height(ui.text_style_height(&TextStyle::Body))
        .shrink_left()
        .show(
            ui,
            |ui| {
                ui.add(Label::new(language.quota_window(&window.label)).wrap());
            },
            |ui| {
                if let Some(reset) = reset {
                    let minutes =
                        (reset.timestamp().saturating_sub(now).max(0) as u64).div_ceil(60);
                    let text = match minutes {
                        0 => language.text("已到重置时间").to_owned(),
                        1..60 => language
                            .format("{minutes} 分钟后重置", &[("minutes", &minutes.to_string())]),
                        60..1440 => language.format(
                            "{hours} 小时 {minutes} 分钟后重置",
                            &[
                                ("hours", &(minutes / 60).to_string()),
                                ("minutes", &(minutes % 60).to_string()),
                            ],
                        ),
                        _ => language.format(
                            "{days} 天 {hours} 小时后重置",
                            &[
                                ("days", &(minutes / 1440).to_string()),
                                ("hours", &(minutes / 60 % 24).to_string()),
                            ],
                        ),
                    };
                    ui.label(RichText::new(text).color(palette.muted).size(12.))
                        .on_hover_text(reset.format("%Y-%m-%d %H:%M UTC").to_string());
                    if ui.is_visible() {
                        ui.ctx().request_repaint_after(Duration::from_secs(60));
                    }
                }
                if let Some(amount) = &amount {
                    let remaining = language.format("剩余 {amount}", &[("amount", amount)]);
                    ui.label(RichText::new(remaining).color(palette.accent).strong());
                }
            },
        );
    if let Some(percent) = window.remaining_percent {
        let target = f32::from(percent) / 100.;
        let progress = if ui.is_visible() {
            ui.ctx()
                .animate_value_with_time(ui.id().with("quota_bar"), target, 0.32)
        } else {
            target
        };
        ui.add(
            egui::ProgressBar::new(progress)
                .desired_height(4.)
                .fill(palette.accent),
        );
    }
}

fn settings_group(ui: &mut Ui, label: &str, content: impl FnOnce(&mut Ui)) {
    let palette = Palette::of(ui);
    surface(palette, palette.paper, Margin::same(18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(label).size(16.).strong());
        ui.add_space(5.);
        content(ui);
    });
}

fn cli_path_settings(ui: &mut Ui, page: &mut SettingsPage, language: Language) -> bool {
    let palette = Palette::of(ui);
    for provider in Provider::ALL {
        ui.push_id(provider, |ui| {
            ui.label(provider.label());
            let path = page.cli_paths.entry(provider).or_default();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let browse = ui.button(language.text("选择文件…"));
                let input = egui::Frame::new()
                    .fill(palette.canvas)
                    .stroke(Stroke::new(1., palette.line))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        input::text_edit(
                            ui,
                            TextEdit::singleline(path)
                                .desired_width(f32::INFINITY)
                                .frame(false)
                                .hint_text(language.text("自动查找")),
                            false,
                        )
                    })
                    .inner;
                if input.changed() {
                    page.cli_path_result = None;
                }
                if browse.clicked()
                    && let Some(file) = rfd::FileDialog::new()
                        .set_title(language.text("选择 CLI 可执行文件"))
                        .pick_file()
                {
                    if let Some(file) = file.to_str() {
                        *path = file.to_owned();
                        page.cli_path_result = None;
                    } else {
                        page.cli_path_result = Some(Err(crate::settings::CliPathError {
                            provider: Some(provider),
                            message: "无法读取此文件名，请选择其他 CLI 文件。".into(),
                        }));
                    }
                }
            });
            ui.add_space(6.);
        });
    }
    ui.horizontal_wrapped(|ui| {
        let save = ui.button(language.text("保存 CLI 路径")).clicked();
        match &page.cli_path_result {
            Some(Ok(())) => {
                ui.colored_label(palette.accent, language.text("已保存 CLI 路径。"));
            }
            Some(Err(error)) => {
                let message = language.text(&error.message);
                if let Some(provider) = error.provider {
                    ui.colored_label(palette.warn, format!("{} · {message}", provider.label()));
                } else {
                    ui.colored_label(palette.warn, message);
                }
            }
            None => {}
        }
        save
    })
    .inner
}

fn proxy_settings(ui: &mut Ui, page: &mut SettingsPage, language: Language) -> bool {
    let palette = Palette::of(ui);
    ui.label(language.text("CLI 网络代理"));
    ui.label(
        RichText::new(
            language.text("用于 Codex、Claude Code 和 Antigravity。留空时沿用启动环境。"),
        )
        .color(palette.muted),
    );
    ui.add_space(8.);
    ui.label(language.text("代理地址"));
    let input = egui::Frame::new()
        .fill(palette.canvas)
        .stroke(Stroke::new(1., palette.line))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            input::text_edit(
                ui,
                TextEdit::singleline(&mut page.proxy_address)
                    .desired_width(f32::INFINITY)
                    .frame(false)
                    .hint_text("http://proxy.example.com:8080"),
                false,
            )
        })
        .inner;
    if input.changed() {
        page.proxy_result = None;
    }
    ui.add_space(4.);
    let mut save = input.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    ui.horizontal_wrapped(|ui| {
        let button = Button::new(
            RichText::new(language.text("保存"))
                .color(palette.on_accent)
                .strong(),
        )
        .fill(palette.accent)
        .stroke(Stroke::NONE)
        .min_size(vec2(76., 34.));
        save |= ui.add(button).clicked();
        match &page.proxy_result {
            Some(Ok(())) => {
                ui.label(
                    RichText::new(language.text("已保存，新的 CLI 连接将使用此设置。"))
                        .color(palette.accent),
                );
            }
            Some(Err(error)) => {
                ui.label(RichText::new(language.text(error)).color(palette.warn));
            }
            None => {}
        }
    });
    save
}

fn centered(ui: &mut Ui, width: f32, render: impl FnOnce(&mut Ui)) {
    let width = width.min(ui.available_width()).max(1.);
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.allocate_ui_with_layout(vec2(width, 0.), Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            render(ui);
        });
    });
}

fn composer_input(
    ui: &mut Ui,
    id: egui::Id,
    max_height: f32,
    composing: bool,
    render: impl FnOnce(&mut Ui),
) {
    let palette = Palette::of(ui);
    ui.scope(|ui| {
        // The bottom panel starts from its previous height. Let the draft measure up to its own cap.
        ui.set_max_height(max_height);
        ui.visuals_mut().text_cursor.stroke.color = palette.composer_text;
        ui.visuals_mut().selection.stroke.color = palette.composer_text;
        ui.visuals_mut().selection.bg_fill = if composing {
            Color32::TRANSPARENT
        } else {
            palette.selection
        };
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(max_height)
            .min_scrolled_height(0.)
            .auto_shrink([false, true])
            .animated(false)
            .show(ui, render);
    });
}

fn composer_tool(ui: &mut Ui, icon: Icon, label: &str, color: Color32) -> egui::Response {
    let button = Button::new(icon.atom())
        .frame_when_inactive(false)
        .stroke(Stroke::NONE)
        .min_size(vec2(32., 34.))
        .atom_ui(ui);
    icon.paint_in(ui, &button, color);
    let response = button.response;
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, response.enabled(), label));
    response.on_hover_text(label)
}

fn model_row(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let palette = Palette::of(ui);
    let text = RichText::new(label).color(palette.ink);
    let mut atoms = egui::Atoms::new((text, Atom::grow()));
    if selected {
        atoms.push_right(Icon::Check.atom());
    }
    let button = Button::new(atoms)
        .min_size(vec2(ui.available_width(), 29.))
        .truncate()
        .selected(selected)
        .frame_when_inactive(selected)
        .atom_ui(ui);
    Icon::Check.paint_in(ui, &button, palette.accent);
    button.response.on_hover_text(label).clicked()
}

const EFFORT_PALETTES: [[Color32; 3]; 5] = [
    [
        Color32::from_rgb(13, 100, 93),
        Color32::from_rgb(33, 167, 141),
        Color32::from_rgb(131, 230, 195),
    ],
    [
        Color32::from_rgb(22, 75, 145),
        Color32::from_rgb(52, 132, 229),
        Color32::from_rgb(145, 202, 250),
    ],
    [
        Color32::from_rgb(50, 33, 115),
        Color32::from_rgb(100, 80, 214),
        Color32::from_rgb(171, 156, 244),
    ],
    [
        Color32::from_rgb(76, 22, 135),
        Color32::from_rgb(142, 43, 228),
        Color32::from_rgb(194, 144, 249),
    ],
    [
        Color32::from_rgb(104, 21, 128),
        Color32::from_rgb(175, 45, 235),
        Color32::from_rgb(224, 160, 251),
    ],
];

fn effort_palette_index(id: &str, index: usize, count: usize) -> usize {
    match id {
        "none" | "minimal" | "low" => 0,
        "medium" => 1,
        "high" => 2,
        "xhigh" => 3,
        "max" => 4,
        _ => (index as f32 / count.saturating_sub(1).max(1) as f32 * 4.).round() as usize,
    }
}

fn paint_effort_capsule(painter: &egui::Painter, rect: Rect, colors: [Color32; 3]) {
    let radius = rect.height().min(rect.width()) * 0.5;
    let mut mesh = egui::epaint::Mesh::default();
    for index in 0..=96 {
        let t = index as f32 / 96.;
        let x = rect.left() + rect.width() * t;
        let edge = (x - (rect.left() + radius).max(x.min(rect.right() - radius))).abs();
        let half = (radius * radius - edge * edge).max(0.).sqrt();
        let color = if t < 0.58 {
            lerp_color(colors[0], colors[1], t / 0.58)
        } else {
            lerp_color(colors[1], colors[2], (t - 0.58) / 0.42)
        };
        mesh.colored_vertex(egui::pos2(x, rect.center().y - half), color);
        mesh.colored_vertex(egui::pos2(x, rect.center().y + half), color);
        if index > 0 {
            let start = (index * 2) as u32;
            mesh.add_triangle(start - 2, start - 1, start);
            mesh.add_triangle(start - 1, start + 1, start);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(0.6, Color32::from_white_alpha(50)),
        StrokeKind::Inside,
    );
}

fn effort_track<'a>(
    ui: &mut Ui,
    choices: &'a [ReasoningChoice],
    selected: Option<&ReasoningChoice>,
) -> Option<&'a ReasoningChoice> {
    let palette = Palette::of(ui);
    if choices.is_empty() {
        return None;
    }
    let language = Language::in_ui(ui);
    let (id, rect) = ui.allocate_space(vec2(ui.available_width(), 94.));
    let slot = rect.width() / choices.len() as f32;
    let selected_index = choices
        .iter()
        .position(|choice| selected.is_some_and(|selected| selected.id == choice.id));
    let last_index = choices.len() - 1;
    let painter = ui.painter();
    let palette_index = selected_index.map_or(0, |index| {
        effort_palette_index(&choices[index].id, index, choices.len())
    });
    let gradient = ui
        .ctx()
        .animate_value_with_time(id.with("palette"), palette_index as f32, 0.28);
    let from = (gradient.floor() as usize).min(4);
    let to = (from + 1).min(4);
    let colors = std::array::from_fn(|index| {
        lerp_color(
            EFFORT_PALETTES[from][index],
            EFFORT_PALETTES[to][index],
            gradient.fract(),
        )
    });
    painter.text(
        rect.min + vec2(6., 10.),
        egui::Align2::LEFT_CENTER,
        language.text("思考强度"),
        FontId::proportional(12.),
        palette.muted,
    );
    painter.text(
        rect.right_top() + vec2(-6., 10.),
        egui::Align2::RIGHT_CENTER,
        selected.map_or("—", |choice| language.text(&choice.label)),
        FontId::proportional(12.),
        colors[1],
    );
    let track = Rect::from_min_size(rect.min + vec2(2., 30.), vec2(rect.width() - 4., 22.));
    painter.rect_filled(track, 11., lerp_color(palette.soft, colors[1], 0.08));
    if let Some(index) = selected_index {
        let position =
            ui.ctx()
                .animate_value_with_time(id.with("effort_index"), index as f32, 0.28);
        let progress = if last_index > 0 {
            position / last_index as f32
        } else {
            1.
        };
        let fill_right = track.left() + track.width() * progress.clamp(0., 1.);
        let thumb_x = fill_right.clamp(rect.left() + 13., rect.right() - 13.);
        let fill = Rect::from_min_max(track.min, egui::pos2(fill_right, track.bottom()));
        if fill.width() > 0.5 {
            paint_effort_capsule(painter, fill, colors);
        }
        let time = ui.input(|input| input.time);
        let duration = 5.6 - f64::from(gradient) * 0.6;
        for index in 0..22 {
            let phase = (index as f64 / 22. - time / duration).rem_euclid(1.) as f32;
            let center = egui::pos2(
                fill.left() + fill.width() * phase,
                fill.top() + fill.height() * (0.18 + (index * 37 % 65) as f32 * 0.01),
            );
            let radius = if index % 5 == 0 { 1.4 } else { 0.75 };
            let capsule_center = egui::pos2(
                center
                    .x
                    .clamp(fill.left() + 11., fill.right().max(fill.left() + 22.) - 11.),
                fill.center().y,
            );
            if center.distance(capsule_center) + radius < 10.5 {
                let alpha = ((phase / 0.12).min((1. - phase) / 0.2).min(1.) * 215.) as u8;
                painter.circle_filled(center, radius + 1.2, Color32::from_white_alpha(alpha / 8));
                painter.circle_filled(center, radius, Color32::from_white_alpha(alpha));
            }
        }
        let center = egui::pos2(thumb_x, track.center().y);
        painter.circle_filled(center, 16., colors[1].gamma_multiply(0.09));
        painter.circle_filled(center + vec2(0., 2.), 13., Color32::from_black_alpha(18));
        painter.circle(center, 13., palette.paper, Stroke::new(1., palette.line));
        if ui.is_rect_visible(rect) {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }
    let mut picked = None;
    for (index, choice) in choices.iter().enumerate() {
        let target =
            Rect::from_min_size(rect.min + vec2(index as f32 * slot, 25.), vec2(slot, 69.));
        let active = selected_index == Some(index);
        let label = language.text(&choice.label);
        let response = ui
            .interact(target, id.with(&choice.id), egui::Sense::click_and_drag())
            .on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, response.enabled(), active, label)
        });
        if response.has_focus() {
            if !response.hovered() && !response.dragged() {
                painter.rect_stroke(
                    target.shrink(1.),
                    6.,
                    Stroke::new(1., colors[1]),
                    StrokeKind::Inside,
                );
            }
            let next = ui.input(|input| {
                if input.key_pressed(egui::Key::Home) {
                    Some(0)
                } else if input.key_pressed(egui::Key::End) {
                    Some(last_index)
                } else if input.key_pressed(egui::Key::ArrowLeft)
                    || input.key_pressed(egui::Key::ArrowDown)
                {
                    Some(index.saturating_sub(1))
                } else if input.key_pressed(egui::Key::ArrowRight)
                    || input.key_pressed(egui::Key::ArrowUp)
                {
                    Some((index + 1).min(last_index))
                } else {
                    None
                }
            });
            if let Some(next) = next {
                picked = Some(&choices[next]);
                ui.memory_mut(|memory| memory.request_focus(id.with(&choices[next].id)));
            }
        }
        let label_t = ui.ctx().animate_bool_with_time(
            id.with(("effort_label", &choice.id)),
            active || response.hovered(),
            0.14,
        );
        painter.text(
            egui::pos2(target.center().x, rect.top() + 74.),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(12.),
            lerp_color(palette.muted, colors[1], label_t),
        );
        if (response.dragged_by(egui::PointerButton::Primary)
            || response.drag_stopped_by(egui::PointerButton::Primary))
            && let Some(pos) = response.interact_pointer_pos()
        {
            let rail = ui
                .input(|input| input.pointer.press_origin())
                .is_some_and(|origin| origin.y < rect.top() + 60.);
            let next = if rail {
                (((pos.x - track.left()) / track.width() * last_index as f32)
                    .round()
                    .max(0.) as usize)
                    .min(last_index)
            } else {
                (((pos.x - rect.left()) / slot).max(0.) as usize).min(last_index)
            };
            picked = Some(&choices[next]);
        } else if response.clicked() {
            let next = response
                .interact_pointer_pos()
                .filter(|pos| pos.y < rect.top() + 60.)
                .map(|pos| {
                    (((pos.x - track.left()) / track.width() * last_index as f32)
                        .round()
                        .max(0.) as usize)
                        .min(last_index)
                })
                .unwrap_or(index);
            picked = Some(&choices[next]);
        }
        response.on_hover_text(label);
    }
    picked
}

fn paint_line(ui: &Ui, origin: egui::Pos2, text: &str, size: f32, color: Color32, width: f32) {
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect(),
        FontId::proportional(size),
        color,
    );
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter().galley(origin, galley, color);
}

fn sidebar_row(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    selected: bool,
    indent: f32,
    running: bool,
) -> bool {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.), egui::Sense::hover());
    let rect = Rect::from_min_max(outer.min + vec2(indent, 0.), outer.max);
    let response = ui.interact(rect, ui.next_auto_id(), egui::Sense::click());
    ui.skip_ahead_auto_ids(1);
    response.widget_info(|| {
        let label = if running {
            language.format("{title}，正在运行", &[("title", title)])
        } else {
            title.to_owned()
        };
        WidgetInfo::labeled(WidgetType::Button, response.enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let row_id = response.id;
        let selected_t = ui
            .ctx()
            .animate_bool_with_time(row_id.with("selected"), selected, 0.14);
        let hover_t = ui.ctx().animate_bool_with_time(
            row_id.with("hover"),
            response.hovered() || response.has_focus(),
            0.12,
        );
        let bg_alpha = selected_t.max(hover_t);
        if bg_alpha > 0.005 {
            let fill = lerp_color(palette.soft, palette.tint, selected_t).gamma_multiply(bg_alpha);
            painter.rect_filled(rect, 7, fill);
        }
        if selected_t > 0.01 {
            let bar_height = 32. * ease_out_cubic(selected_t);
            let bar_top = rect.center().y - bar_height * 0.5;
            painter.rect_filled(
                Rect::from_min_size(egui::pos2(rect.left() + 2., bar_top), vec2(2., bar_height)),
                1,
                palette.accent.gamma_multiply(selected_t),
            );
        }
        let title = if title.trim().is_empty() {
            language.text("未命名会话")
        } else {
            title
        };
        let x = rect.left() + 9.;
        let mut right = rect.right() - 9.;
        if running {
            let spinner =
                Rect::from_center_size(egui::pos2(right - 9., rect.center().y), vec2(18., 18.));
            egui::Spinner::new()
                .color(palette.accent)
                .paint_at(ui, spinner);
            right = spinner.left() - 7.;
        }
        let w = (right - x).max(1.);
        paint_line(
            ui,
            egui::pos2(x, rect.top() + 6.),
            title,
            13.,
            palette.ink,
            w,
        );
        paint_line(
            ui,
            egui::pos2(x, rect.top() + 28.),
            detail,
            11.,
            palette.muted,
            w,
        );
    }
    response
        .on_hover_text(format!("{title}\n{detail}"))
        .clicked()
}

#[cfg(test)]
mod native_restoration_tests {
    use super::*;

    #[test]
    fn compact_more_button_and_action_rows_fit_all_languages() {
        for language in Language::ALL {
            for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
                let ctx = egui::Context::default();
                setup(&ctx);
                ctx.set_theme(theme);
                language.apply(&ctx);
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(760., 548.))),
                        ..Default::default()
                    },
                    |ctx| {
                        window_header(
                            ctx,
                            "compact native header test",
                            20,
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                let button = Icon::More.button(
                                    ui,
                                    language.text("会话操作"),
                                    Palette::of(ui).muted,
                                );
                                assert!(button.rect.width() <= 38.);
                                assert!(button.rect.height() < TITLE_BAR_HEIGHT);
                                assert!(ui.max_rect().contains_rect(button.rect));
                            },
                        );
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.allocate_ui_with_layout(
                                vec2(204., 0.),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_width(204.);
                                    for label in [
                                        "上下文用量",
                                        "重命名",
                                        "创建分支",
                                        "目标",
                                        "文件回退",
                                        "扩展状态",
                                    ] {
                                        let start = ui.cursor().min;
                                        action_menu_row(ui, label);
                                        assert!(
                                            ui.min_rect().width() <= 205.,
                                            "{}: {label}",
                                            language.code()
                                        );
                                        assert!(ui.cursor().min.y >= start.y + 30.);
                                    }
                                },
                            );
                        });
                    },
                );
            }
        }
    }

    #[test]
    fn startup_and_readable_status_surfaces_fit_narrow_modal_columns() {
        for language in Language::ALL {
            let ctx = egui::Context::default();
            setup(&ctx);
            language.apply(&ctx);
            for provider in Provider::ALL {
                let mut draft = StartupDraft::default();
                let _ = ctx.run(egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(760., 548.))),
                    ..Default::default()
                }, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.allocate_ui_with_layout(vec2(300., 0.), Layout::top_down(Align::Min), |ui| {
                            ui.set_width(300.);
                            startup_settings(ui, &mut draft, provider, true);
                            render_context_details(ui, None);
                            render_rewind_preview(ui, &serde_json::json!({"filesChanged":["/workspace/src/main.rs"],"insertions":2,"deletions":1}));
                            assert!(ui.min_rect().width() <= 301., "{} {}: {:?}", language.code(), provider.label(), ui.min_rect());
                        });
                    });
                });
            }
        }
    }

    #[test]
    fn context_details_distinguish_unknown_from_zero() {
        let ctx = egui::Context::default();
        setup(&ctx);
        Language::English.apply(&ctx);
        let report = engine::work::ContextUsage::new(Some(0), None);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_context_details(ui, Some(&report));
            });
        });
        let labels = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        });
        assert!(labels.clone().any(|label| label == "Used · 0"));
        assert!(
            labels
                .clone()
                .any(|label| label == "Limit · Not reported by engine")
        );
    }
}

#[cfg(test)]
mod effort_tests {
    use super::*;

    #[test]
    fn effort_endpoints_fill_the_entire_rail_and_keep_thumb_inside() {
        for count in [1, 2, 3, 4, 5] {
            for width in [240., 285., 340.] {
                for index in [0, count - 1] {
                    let ctx = egui::Context::default();
                    setup(&ctx);
                    let choices: Vec<ReasoningChoice> = (0..count)
                        .map(|index| ReasoningChoice {
                            id: format!("effort-{index}"),
                            label: format!("Level {index}"),
                        })
                        .collect();
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                egui::Pos2::ZERO,
                                vec2(400., 240.),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                ui.set_width(width);
                                effort_track(ui, &choices, Some(&choices[index]));
                            });
                        },
                    );
                    let track = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect) if (rect.rect.height() - 22.).abs() < 0.01 => {
                                Some(rect.rect)
                            }
                            _ => None,
                        })
                        .expect("rail should be painted");
                    let thumb = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Circle(circle)
                                if circle.radius == 13. && circle.fill == Palette::LIGHT.paper =>
                            {
                                Some(circle)
                            }
                            _ => None,
                        })
                        .expect("white thumb should be painted");
                    assert!(thumb.center.x - thumb.radius >= track.left() - 2.01);
                    assert!(thumb.center.x + thumb.radius <= track.right() + 2.01);
                    let gradient = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) if mesh.vertices.len() > 100 => Some(mesh),
                        _ => None,
                    });
                    if index == count - 1 {
                        let mesh = gradient.expect("maximum effort should fill the rail");
                        let left = mesh
                            .vertices
                            .iter()
                            .map(|vertex| vertex.pos.x)
                            .fold(f32::INFINITY, f32::min);
                        let right = mesh
                            .vertices
                            .iter()
                            .map(|vertex| vertex.pos.x)
                            .fold(f32::NEG_INFINITY, f32::max);
                        assert!((left - track.left()).abs() < 0.01);
                        assert!(
                            (right - track.right()).abs() < 0.01,
                            "maximum effort must reach the right edge"
                        );
                    } else {
                        assert!(gradient.is_none(), "minimum effort must start at zero");
                    }
                }
            }
        }
    }

    #[test]
    fn effort_meter_animates_and_reply_text_stays_around_work() {
        let ctx = egui::Context::default();
        setup(&ctx);
        let choices: Vec<ReasoningChoice> = ["low", "medium", "high", "max"]
            .into_iter()
            .map(Into::into)
            .collect();
        let render = |time| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360., 400.))),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(285.);
                        effort_track(ui, &choices, Some(&choices[2]));
                    });
                },
            )
        };
        let first = render(0.);
        let second = render(0.4);
        let particles = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Circle(circle) = &shape.shape {
                        (circle.radius <= 3.).then_some(circle.center)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert!(!particles(&first).is_empty());
        assert_ne!(
            particles(&first),
            particles(&second),
            "white particles should move with time"
        );
        assert!(
            first.shapes.iter().any(
                |shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.len() > 100)
            ),
            "capsule should have a gradient fill"
        );
        assert!(first.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(circle) if circle.radius == 13. && circle.fill == Palette::LIGHT.paper)), "slider should have a round white thumb");
        assert_eq!(
            effort_palette_index("high", 2, 4),
            effort_palette_index("high", 2, 5)
        );
        assert_ne!(
            effort_palette_index("high", 2, 5),
            effort_palette_index("max", 4, 5)
        );
        let mut work = Work::default();
        work.record(Step {
            id: "tool".into(),
            kind: StepKind::Tool,
            status: StepStatus::Complete,
            after: None,
            text_offset: Some(6),
            text: "Read".into(),
            exit_code: None,
        });
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut files = files::FileViewer::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(360., 400.))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_reply(
                        ui,
                        &mut cache,
                        &mut files,
                        "BeforeAfter",
                        &work,
                        ReplyFileLinks::Local(Path::new("/tmp")),
                    );
                });
            },
        );
        let top = |value: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        (text.galley.text() == value).then_some(text.pos.y)
                    } else {
                        None
                    }
                })
                .expect(value)
        };
        assert!(top("Before") < top("工作过程"));
        assert!(top("工作过程") < top("After"));
    }
}

#[cfg(test)]
mod composer_tests {
    use super::*;

    #[test]
    fn draft_reserves_two_rows_then_grows_and_shrinks_with_text() {
        let ctx = egui::Context::default();
        setup(&ctx);
        let measure = |text: &str| {
            let mut draft = text.to_owned();
            let mut height = 0.;
            for _ in 0..2 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(780., 620.))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::TopBottomPanel::bottom("composer test").show(ctx, |ui| {
                            centered(ui, 240., |ui| {
                                height = ui
                                    .scope(|ui| {
                                        composer_input(
                                            ui,
                                            egui::Id::new("test draft"),
                                            120.,
                                            false,
                                            |ui| {
                                                ui.add(
                                                    TextEdit::multiline(&mut draft)
                                                        .desired_rows(COMPOSER_MIN_EDITOR_ROWS)
                                                        .desired_width(f32::INFINITY),
                                                );
                                            },
                                        );
                                    })
                                    .response
                                    .rect
                                    .height();
                            });
                        });
                    },
                );
            }
            height
        };
        let empty = measure("");
        assert_eq!(measure("one"), empty);
        // The laid-out text can round to a slightly different height than the reserved rows.
        assert!((measure("one\ntwo") - empty).abs() < 1.);
        assert!(measure("one\ntwo\nthree") > empty + 10.);
        assert!(measure(&"a soft-wrapped line ".repeat(5)) > empty + 10.);
        assert_eq!(measure(&"long draft\n".repeat(60)), 120.);
        assert_eq!(measure(""), empty);
    }
}

#[cfg(test)]
mod device_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn device_dialog_stays_compact_and_pairing_remains_reachable() {
        for language in Language::ALL {
            for size in [vec2(760., 548.), vec2(1920., 997.)] {
                let screen = Rect::from_min_size(egui::Pos2::ZERO, size);
                let ctx = egui::Context::default();
                setup(&ctx);
                language.apply(&ctx);
                let host = "https://naro.example.com/devices/00112233445566778899aabbccddeeff/";
                let qr = PairingQr::new(&ctx, pairing_link(host, "12345678"));
                let address = RefCell::new(String::new());
                let code = RefCell::new(String::new());
                let pair = Cell::new(false);
                let tab = Cell::new(DeviceTab::default());
                let bounds = Cell::new(Rect::NOTHING);
                let render = |events| {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            events,
                            ..Default::default()
                        },
                        |ctx| {
                            let mut selected = tab.get();
                            let modal =
                                show_device_modal(
                                    ctx,
                                    &mut selected,
                                    true,
                                    "",
                                    1.,
                                    |ui, tab| match tab {
                                        DeviceTab::Connect => {
                                            let palette = Palette::of(ui);
                                            surface(palette, palette.canvas, Margin::same(14))
                                                .show(ui, |ui| {
                                                    ui.set_width(ui.available_width());
                                                    pair.set(remote_pairing_form(
                                                        ui,
                                                        &mut address.borrow_mut(),
                                                        &mut code.borrow_mut(),
                                                        &mut None,
                                                        false,
                                                        true,
                                                    ));
                                                });
                                        }
                                        DeviceTab::PairThisComputer => {
                                            pairing_card(ui, qr.texture.as_ref(), host, "12345678");
                                            ui.add_space(4.);
                                            egui::CollapsingHeader::new(language.text("中继设置"))
                                                .show(ui, |_| {});
                                        }
                                    },
                                    |ui| {
                                        let _ = ui.button(language.text("打开本机网页"));
                                        let _ = ui.button(language.text("打开中继网页"));
                                    },
                                );
                            tab.set(selected);
                            bounds.set(modal.response.rect);
                        },
                    )
                };

                let settled = || {
                    // The oversized row grew as egui reused the prior frame's area.
                    for _ in 0..5 {
                        render(vec![]);
                    }
                    let output = render(vec![]);
                    let max_height = match tab.get() {
                        DeviceTab::Connect => 350.,
                        DeviceTab::PairThisComputer => 440.,
                    };
                    assert!(
                        screen.shrink(12.).contains_rect(bounds.get())
                            && bounds.get().height() < max_height,
                        "{} {} modal {:?} in {screen:?}",
                        language.code(),
                        tab.get().label(),
                        bounds.get(),
                    );
                    output
                };
                let text_rect = |output: &egui::FullOutput, label| {
                    output
                        .shapes
                        .iter()
                        .find_map(|clipped| match &clipped.shape {
                            egui::Shape::Text(text)
                                if text.galley.text() == language.text(label) =>
                            {
                                let rect = text.visual_bounding_rect();
                                clipped
                                    .clip_rect
                                    .expand(1.)
                                    .contains_rect(rect)
                                    .then_some(rect)
                            }
                            _ => None,
                        })
                };
                let visible_text = |output: &egui::FullOutput, label| {
                    text_rect(output, label)
                        .unwrap_or_else(|| panic!("Hidden device dialog label: {label}"))
                };
                let outgoing = settled();
                let code_hint = visible_text(&outgoing, "8 位数字");
                let button = visible_text(&outgoing, "配对并连接");
                assert!((button.center().y - code_hint.center().y).abs() < 4.);
                assert!(code_hint.right() < button.left());
                assert!(text_rect(&outgoing, "手机扫码连接").is_none());
                assert!(text_rect(&outgoing, "打开本机网页").is_none());

                let click = |target: Rect| {
                    let event = |pressed| egui::Event::PointerButton {
                        pos: target.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    };
                    render(vec![
                        egui::Event::PointerMoved(target.center()),
                        event(true),
                        event(false),
                    ])
                };
                click(button);
                assert!(!pair.get());
                *address.borrow_mut() = host.into();
                *code.borrow_mut() = "12345678".into();
                render(vec![]);
                click(button);
                assert!(
                    pair.get(),
                    "pair button must be clickable after entering credentials"
                );

                click(visible_text(&outgoing, DeviceTab::PairThisComputer.label()));
                assert!(tab.get() == DeviceTab::PairThisComputer);
                let incoming = settled();
                assert!(text_rect(&incoming, "配对并连接").is_none());
                assert!(text_rect(&incoming, "设备地址").is_none());
                visible_text(&incoming, "本机在线");
                visible_text(&incoming, "手机扫码连接");
                assert!(
                    visible_text(&incoming, "手动配对").bottom()
                        < visible_text(&incoming, "打开本机网页").top()
                );
                visible_text(&incoming, "中继设置");
                visible_text(&incoming, "打开中继网页");
                click(visible_text(&incoming, DeviceTab::Connect.label()));
                assert!(tab.get() == DeviceTab::Connect);
                visible_text(&settled(), "配对并连接");
            }
        }
    }
}

#[cfg(test)]
mod quota_tests {
    use super::*;

    #[test]
    fn quota_windows_align_status_to_the_right_edge() {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(640., 160.));
        for language in Language::ALL {
            for (width, label) in [(294., "Long model limit · 7 天"), (572., "7 天")] {
                for resets_at in [None, Some((4 * 24 + 18) * 3600)] {
                    let ctx = egui::Context::default();
                    setup(&ctx);
                    language.apply(&ctx);
                    let window = engine::QuotaWindow {
                        label: label.to_owned(),
                        remaining_percent: Some(63),
                        remaining_amount: None,
                        resets_at,
                    };
                    let mut left = 0.;
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                ui.allocate_ui_with_layout(
                                    vec2(width, 0.),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        left = ui.max_rect().left();
                                        render_quota_window(ui, &window, 0);
                                    },
                                );
                            });
                        },
                    );
                    let bounds = |label: &str| {
                        output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Text(text) if text.galley.text() == label => {
                                    Some(text.visual_bounding_rect())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| panic!("Missing quota label: {label}"))
                    };
                    let period = bounds(&language.quota_window(label));
                    let amount = bounds(&language.format("剩余 {amount}", &[("amount", "63%")]));
                    let mut right = amount.right();
                    if resets_at.is_some() {
                        let reset = bounds(&language.format(
                            "{days} 天 {hours} 小时后重置",
                            &[("days", "4"), ("hours", "18")],
                        ));
                        assert!(amount.right() < reset.left());
                        assert!((amount.center().y - reset.center().y).abs() < 4.);
                        right = reset.right();
                    }
                    assert!((period.left() - left).abs() < 3.);
                    assert!(period.right() < amount.left());
                    assert!(
                        (right - left - width).abs() < 3.,
                        "status ends at {right}, expected {}",
                        left + width
                    );
                }
            }
        }
    }

    #[test]
    fn quota_modal_fits_long_lists_and_keeps_controls_clickable() {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(760., 548.));
        let quota = Quota {
            limits: (0..4)
                .map(|group| engine::QuotaLimit {
                    name: format!("Limit group {group}"),
                    windows: (0..5)
                        .map(|row| engine::QuotaWindow {
                            label: format!("Model {group}-{row} · 7 天"),
                            remaining_percent: Some(40),
                            remaining_amount: None,
                            resets_at: Some(i64::from(i32::MAX)),
                        })
                        .collect(),
                })
                .collect(),
            ..Default::default()
        };
        for language in Language::ALL {
            let ctx = egui::Context::default();
            setup(&ctx);
            language.apply(&ctx);
            let clicked = std::cell::Cell::new(false);
            let closed = std::cell::Cell::new(false);
            let render = |events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        let modal = show_quota_modal(ctx, |ui| {
                            render_quota(ui, &quota);
                            let button = ui.button(language.text("确认重置"));
                            if ui.is_visible() {
                                let transform =
                                    ctx.layer_transform_to_global(ui.layer_id()).unwrap();
                                assert!(transform.scaling < 1.);
                                assert!(
                                    screen.shrink(15.).contains_rect(transform * ui.min_rect())
                                );
                            }
                            button.clicked()
                        });
                        clicked.set(modal.inner);
                        closed.set(modal.should_close());
                    },
                )
            };
            render(vec![]);
            let output = render(vec![]);
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => {
                        Some((text.galley.text(), text.visual_bounding_rect()))
                    }
                    _ => None,
                })
                .collect();
            for group in &quota.limits {
                for row in &group.windows {
                    let label = language.quota_window(&row.label);
                    assert!(
                        labels
                            .iter()
                            .any(|(text, rect)| *text == label && screen.contains_rect(*rect)),
                        "Missing or clipped quota label: {label}"
                    );
                }
            }
            let click = |position| {
                let button = |pressed| egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                vec![
                    egui::Event::PointerMoved(position),
                    button(true),
                    button(false),
                ]
            };
            let button_label = language.text("确认重置");
            let button_rect = labels
                .iter()
                .find(|(text, _)| *text == button_label)
                .unwrap()
                .1;
            render(click(button_rect.center()));
            assert!(clicked.get());
            assert!(!closed.get());
            render(click(egui::pos2(4., 4.)));
            assert!(closed.get());
        }
    }

    #[test]
    fn token_usage_modal_hierarchy_renders_and_switches_controls() {
        assert_eq!(format_share(0, 100), None);
        assert_eq!(format_share(10, 0), None);
        assert_eq!(format_share(1, 10_000), Some("<0.1%".into()));
        assert_eq!(format_share(50, 100), Some("50%".into()));
        assert_eq!(format_share(224_880_000, 398_040_000), Some("56.5%".into()));

        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(760., 700.));
        let window = engine::usage::UsageWindow {
            period: UsagePeriod::Day,
            total: Some(TokenCounts {
                input_tokens: 396_470_000,
                output_tokens: 1_570_000,
                cache_read_tokens: 388_600_000,
                cache_write_tokens: 0,
            }),
            providers: [
                engine::usage::ProviderUsage {
                    provider: Provider::Codex,
                    counts: Some(TokenCounts {
                        input_tokens: 223_840_000,
                        output_tokens: 1_040_000,
                        cache_read_tokens: 218_970_000,
                        cache_write_tokens: 0,
                    }),
                    partial: false,
                },
                engine::usage::ProviderUsage {
                    provider: Provider::Claude,
                    counts: Some(TokenCounts {
                        input_tokens: 167_310_000,
                        output_tokens: 440_034,
                        cache_read_tokens: 164_810_000,
                        cache_write_tokens: 0,
                    }),
                    partial: false,
                },
                engine::usage::ProviderUsage {
                    provider: Provider::Antigravity,
                    counts: Some(TokenCounts {
                        input_tokens: 5_320_000,
                        output_tokens: 92_125,
                        cache_read_tokens: 4_820_000,
                        cache_write_tokens: 0,
                    }),
                    partial: true,
                },
            ],
        };

        for language in Language::ALL {
            let ctx = egui::Context::default();
            setup(&ctx);
            language.apply(&ctx);
            let showing_account = std::cell::Cell::new(false);
            let period = std::cell::Cell::new(UsagePeriod::Day);
            let render = |events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        show_quota_modal(ctx, |ui| {
                            let mut account = showing_account.get();
                            usage_section_tabs(ui, &mut account);
                            showing_account.set(account);
                            ui.add_space(10.);
                            let periods =
                                UsagePeriod::ALL.map(|item| (item, language.text(item.label())));
                            if let Some(next) = segmented_bar(ui, period.get(), &periods) {
                                period.set(next);
                            }
                            ui.add_space(10.);
                            render_token_usage_window(ui, &window);
                            false
                        });
                    },
                )
            };
            render(vec![]);
            let output = render(vec![]);
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => {
                        Some((text.galley.text().to_owned(), text.visual_bounding_rect()))
                    }
                    _ => None,
                })
                .collect();
            for expected in [
                language.text("Token 用量"),
                language.text("账号额度"),
                language.text("总消耗"),
                language.text("各引擎明细"),
                "Codex",
                "Claude Code",
                "Antigravity",
                "56.5%",
                "42.1%",
                "1.4%",
            ] {
                assert!(
                    labels
                        .iter()
                        .any(|(text, rect)| text == expected && screen.contains_rect(*rect)),
                    "Missing or clipped usage label: {expected}"
                );
            }
            let click = |position| {
                let button = |pressed| egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                vec![
                    egui::Event::PointerMoved(position),
                    button(true),
                    button(false),
                ]
            };
            let week_label = language.text(UsagePeriod::Week.label());
            let week_rect = labels
                .iter()
                .find(|(text, _)| text == week_label)
                .unwrap()
                .1;
            render(click(week_rect.center()));
            assert_eq!(period.get(), UsagePeriod::Week);

            let account_label = language.text("账号额度");
            let account_rect = labels
                .iter()
                .find(|(text, _)| text == account_label)
                .unwrap()
                .1;
            render(click(account_rect.center()));
            assert!(showing_account.get());
        }
    }
}

#[cfg(test)]
mod sidebar_tests {
    use super::*;

    #[test]
    fn sidebar_rows_keep_text_separated_and_inside_padding() {
        let title = "App内打开文件并支持MD渲染与源码切换";
        let detail = "Claude Code";
        for scale in [1., 2.] {
            for (indent, running) in [(0., true), (17., false)] {
                let ctx = egui::Context::default();
                setup(&ctx);
                let mut input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(240., 120.))),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                let mut row = Rect::NOTHING;
                let output = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        row = ui
                            .scope(|ui| {
                                sidebar_row(ui, title, detail, true, indent, running);
                            })
                            .response
                            .rect;
                    });
                });
                let text_shape = |value: &str| {
                    output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.text() == value => Some(text),
                            _ => None,
                        })
                        .expect("sidebar text should be painted")
                };
                let title_shape = text_shape(title);
                let detail_shape = text_shape(detail);
                let title_bounds = title_shape.visual_bounding_rect();
                let detail_bounds = detail_shape.visual_bounding_rect();
                assert_eq!(row.height(), 50.);
                assert_eq!(title_shape.galley.rows.len(), 1);
                assert!(
                    title_shape.galley.elided,
                    "long titles should truncate: scale={scale}, indent={indent}, row={row:?}, title={title_bounds:?}"
                );
                assert!(title_bounds.top() >= row.top() + 5.);
                assert!(detail_bounds.top() - title_bounds.bottom() >= 4.);
                assert!(detail_bounds.bottom() <= row.bottom() - 5.);
                let left = row.left() + indent + 9.;
                let right = row.right() - 9. - if running { 25. } else { 0. };
                for bounds in [title_bounds, detail_bounds] {
                    // Glyph bearings can extend slightly past their layout box.
                    assert!(bounds.left() >= left - 1.);
                    assert!(bounds.right() <= right + 1.);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum ReplyFileLinks<'a> {
    Local(&'a Path),
    Remote(&'a remote::RemoteClient, &'a BTreeMap<String, String>),
}

impl<'a> ReplyFileLinks<'a> {
    fn new(cwd: &'a Path, client: Option<&'a remote::RemoteClient>, work: &'a Work) -> Self {
        match client {
            Some(client) => Self::Remote(client, &work.remote_file_links),
            None => Self::Local(cwd),
        }
    }
}

fn render_progress_note(
    ui: &mut Ui,
    cache: &mut egui_commonmark::CommonMarkCache,
    files: &mut files::FileViewer,
    id: &str,
    text: &str,
    links: ReplyFileLinks<'_>,
) {
    if !text.is_empty() {
        ui.add_space(8.);
        ui.push_id(("progress", id), |ui| {
            render_document(ui, cache, files, text, links)
        });
    }
}

fn render_document(
    ui: &mut Ui,
    cache: &mut egui_commonmark::CommonMarkCache,
    files: &mut files::FileViewer,
    text: &str,
    links: ReplyFileLinks<'_>,
) {
    match links {
        ReplyFileLinks::Local(cwd) => {
            if let Some(path) = markdown::render(ui, cache, text, cwd) {
                files.open(ui.ctx(), path, cwd);
            }
        }
        ReplyFileLinks::Remote(client, links) => {
            if let Some(key) = markdown::render_remote(ui, cache, text, links) {
                files.open_remote(ui.ctx(), client, key);
            }
        }
    }
}

fn local_file_chip(ui: &mut Ui, file: &LocalAttachment, removable: bool) -> egui::Response {
    let palette = Palette::of(ui);
    let icon = if file.directory {
        Icon::Folder
    } else {
        Icon::Document
    };
    let mut atoms = Atoms::new((icon.atom(), RichText::new(file.name()).size(12.)));
    if removable {
        atoms.push_right(Icon::Close.atom());
    }
    let button = Button::new(atoms).truncate();
    let button = if removable {
        button.small().min_size(vec2(0., 25.)).corner_radius(7.)
    } else {
        button
            .min_size(vec2(0., 30.))
            .corner_radius(9.)
            .fill(palette.canvas)
            .stroke(Stroke::new(1., palette.line))
            .sense(egui::Sense::hover())
    };
    let button = button.atom_ui(ui);
    icon.paint_in(ui, &button, palette.muted);
    if removable {
        Icon::Close.paint_in(ui, &button, palette.muted);
        let language = Language::in_ui(ui);
        button.response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Button,
                button.response.enabled(),
                format!("{} · {}", file.name(), language.text("点击移除")),
            )
        });
    }
    button.response.on_hover_text(&file.path)
}

fn history_images<'a>(
    attachments: &'a [UserAttachment],
    remote: Option<&'a remote::RemoteClient>,
) -> Vec<PromptImage<'a>> {
    attachments
        .iter()
        .map(|attachment| PromptImage {
            name: &attachment.name,
            source: attachment
                .image
                .as_ref()
                .and_then(|image| ThumbnailSource::history(image, remote)),
        })
        .collect()
}

fn run_images<'a>(
    run: Option<&Run>,
    remote: Option<(&'a remote::RemoteClient, &'a str)>,
    names: &'a [String],
    start: usize,
) -> Vec<PromptImage<'a>> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| PromptImage {
            name,
            source: start.checked_add(index).and_then(|index| {
                if let Some((client, id)) = remote {
                    Some(ThumbnailSource::Remote(client, RemoteImage::Run(id, index)))
                } else {
                    run.and_then(|run| run.image(index))
                        .map(ThumbnailSource::Sent)
                }
            }),
        })
        .collect()
}

fn render_prompt_files(ui: &mut Ui, files: &[LocalAttachment]) {
    if !files.is_empty() {
        ui.with_layout(
            Layout::right_to_left(Align::Min).with_main_wrap(true),
            |ui| {
                ui.spacing_mut().item_spacing = vec2(6., 5.);
                for file in files.iter().rev() {
                    local_file_chip(ui, file, false);
                }
            },
        );
    }
}

fn render_prompt(ui: &mut Ui, text: &str) {
    let palette = Palette::of(ui);
    let display = text.trim_end();
    if display.is_empty() {
        return;
    }
    let padding = Margin::symmetric(15, 10);
    let max_width = (ui.available_width() * 0.82 - padding.sum().x).clamp(1., 580.);
    let body_font = TextStyle::Body.resolve(ui.style());
    let width = ui.fonts_mut(|fonts| {
        let width = fonts
            .layout(display.to_owned(), body_font, palette.ink, max_width)
            .size()
            .x;
        (width.ceil() + 1.).min(max_width)
    });
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.allocate_ui_with_layout(
            vec2(width + padding.sum().x, 0.),
            Layout::top_down(Align::Min),
            |ui| {
                egui::Frame::new()
                    .fill(palette.soft)
                    .inner_margin(padding)
                    .corner_radius(13)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.add(Label::new(display).wrap().selectable(true));
                    });
                ui.add_space(3.);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    copy_message(ui, text)
                });
            },
        );
    });
}

fn render_reply(
    ui: &mut Ui,
    cache: &mut egui_commonmark::CommonMarkCache,
    files: &mut files::FileViewer,
    text: &str,
    work: &Work,
    links: ReplyFileLinks<'_>,
) {
    let language = Language::in_ui(ui);
    if !work.plan.is_empty() {
        ui.strong(language.text("当前计划"));
        for step in &work.plan {
            let status = match step.status.as_str() {
                "pending" => "待办",
                "inProgress" => "进行中",
                "completed" => "已完成",
                other => other,
            };
            ui.label(format!("{} · {}", language.text(status), step.step));
        }
    }
    if !work.agents.is_empty() {
        render_agent_tasks(ui, &work.agents);
    }
    for part in work.reply_parts(text) {
        match part {
            ReplyPart::Text { offset, text } if !text.trim().is_empty() => {
                ui.add_space(8.);
                ui.push_id(("reply text", offset), |ui| {
                    render_document(ui, cache, files, text, links);
                });
            }
            ReplyPart::Steps(steps) => {
                ui.push_id(("reply steps", &steps[0].id), |ui| {
                    render_work_steps(ui, cache, files, steps, links)
                });
            }
            _ => {}
        }
    }
    if !work.changes.is_empty() {
        render_changes(ui, &work.changes);
    }
    if !text.trim().is_empty() {
        copy_message(ui, text);
    }
}

fn copy_message(ui: &mut Ui, text: &str) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let copy_id = ui.id().with("copied_at");
    let now = ui.input(|input| input.time);
    let copied_at = ui.ctx().data(|data| data.get_temp::<f64>(copy_id));
    let copied = copied_at.is_some_and(|at| now - at < 1.6);
    if let Some(at) = copied_at
        && now - at < 1.6
    {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64((1.6 - (now - at)).max(0.05)));
    }
    let copied_t = ui
        .ctx()
        .animate_bool_with_time(copy_id.with("anim"), copied, 0.14);
    let color = lerp_color(palette.muted, palette.accent, copied_t);
    let icon = if copied { Icon::Check } else { Icon::Copy };
    let label = if copied {
        language.text("已复制")
    } else {
        language.text("复制")
    };
    if icon.button(ui, label, color).clicked() {
        ui.ctx().copy_text(text.to_owned());
        ui.ctx().data_mut(|data| data.insert_temp(copy_id, now));
        ui.ctx().request_repaint();
    }
}

fn render_agent_tasks(ui: &mut Ui, agents: &[Agent]) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    ui.add_space(6.);
    surface(palette, palette.canvas, Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(language.text("子代理"))
                .color(palette.muted)
                .size(12.)
                .strong(),
        );
        egui::ScrollArea::vertical()
            .id_salt("agent tasks")
            .max_height(220.)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, agent) in agents.iter().enumerate() {
                    if index > 0 {
                        turn_divider(ui);
                    }
                    let (status, color) = match agent.status {
                        Some(AgentStatus::Running) => ("进行中", palette.accent),
                        Some(AgentStatus::Started) => ("已启动", palette.accent),
                        Some(AgentStatus::Complete) => ("已完成", palette.added),
                        Some(AgentStatus::Failed) => ("未成功", palette.danger),
                        Some(AgentStatus::Interrupted) => ("已中断", palette.warn),
                        None => ("状态待确认", palette.muted),
                    };
                    let fallback =
                        language.format("子代理 {count}", &[("count", &(index + 1).to_string())]);
                    let (name, description) = if !agent.name.is_empty() {
                        (agent.name.as_str(), agent.description.as_str())
                    } else if !agent.description.is_empty() {
                        (agent.description.as_str(), "")
                    } else {
                        (fallback.as_str(), "")
                    };
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.add(
                            Label::new(
                                RichText::new(language.text(status)).size(11.5).color(color),
                            )
                            .wrap(),
                        );
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.add(Label::new(RichText::new(name).size(13.).strong()).wrap());
                            if !description.is_empty() {
                                ui.add(
                                    Label::new(
                                        RichText::new(description).size(12.).color(palette.muted),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                    });
                }
            });
    });
}

fn render_work_steps(
    ui: &mut Ui,
    cache: &mut egui_commonmark::CommonMarkCache,
    files: &mut files::FileViewer,
    steps: &[Step],
    links: ReplyFileLinks<'_>,
) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let steps = steps.iter().filter(|step| match step.kind {
        StepKind::Note => false,
        StepKind::Reasoning | StepKind::Thinking => !step.text.trim().is_empty(),
        _ => !step.text.is_empty() || step.status == StepStatus::Running,
    });
    let count = steps.clone().count();
    if count == 0 {
        return;
    }
    let only_commands = steps.clone().all(|step| step.kind == StepKind::Command);
    let running = steps
        .clone()
        .rfind(|step| step.status == StepStatus::Running);
    let summary = match running.map(|step| step.kind) {
        Some(StepKind::Command) => language.text("正在运行命令"),
        Some(StepKind::Tool) => language.text("正在使用工具"),
        Some(StepKind::Reasoning | StepKind::Thinking) => language.text("正在思考"),
        _ if only_commands => language.text("运行了命令"),
        _ if steps.clone().all(|step| step.kind.is_thinking()) => language.text("思考"),
        _ => language.text("工作过程"),
    };
    let icon = if only_commands || running.is_some_and(|step| step.kind == StepKind::Command) {
        Icon::Terminal
    } else {
        Icon::Lightning
    };

    ui.add_space(4.);
    disclosure(
        ui,
        "work summary",
        |ui, openness| {
            let mut atoms = Atoms::new((icon.atom(), RichText::new(summary).size(13.).strong()));
            if count > 1 {
                atoms.push_right(
                    RichText::new(language.format("{count} 项", &[("count", &count.to_string())]))
                        .size(11.5)
                        .color(palette.muted),
                );
            }
            let button = disclosure_button(ui, atoms, 0., openness, true);
            icon.paint_in(ui, &button, palette.accent);
            button.response
        },
        |ui| {
            surface(palette, palette.paper, Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("work steps")
                    .max_height(200.)
                    .min_scrolled_height(200.)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (index, step) in steps.enumerate() {
                            if index > 0 {
                                turn_divider(ui);
                            }
                            let label = match step.kind {
                                StepKind::Note => language.text("进度更新"),
                                StepKind::Reasoning | StepKind::Thinking => language.text("思考"),
                                StepKind::Command => language.text("命令"),
                                StepKind::Tool => language.text("工具"),
                            };
                            let (status, color) = match step.status {
                                StepStatus::Running => (language.text("进行中"), palette.accent),
                                StepStatus::Complete => (language.text("已完成"), palette.muted),
                                StepStatus::Failed => (language.text("未成功"), palette.danger),
                                StepStatus::Interrupted => (language.text("已中断"), palette.warn),
                            };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(label).size(13.).strong());
                                ui.label(RichText::new(status).size(11.5).color(color));
                            });
                            if step.text.is_empty() {
                                continue;
                            }
                            if step.kind == StepKind::Command {
                                egui::Frame::new()
                                    .fill(palette.soft)
                                    .corner_radius(6)
                                    .inner_margin(Margin::symmetric(9, 6))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.add(
                                            Label::new(RichText::new(&step.text).monospace())
                                                .wrap()
                                                .selectable(true),
                                        );
                                    });
                            } else if step.kind.is_thinking() {
                                ui.push_id(&step.id, |ui| {
                                    ui.style_mut().override_font_id =
                                        Some(egui::FontId::proportional(13.));
                                    render_document(ui, cache, files, &step.text, links);
                                });
                            } else {
                                ui.add(
                                    Label::new(RichText::new(&step.text).size(13.))
                                        .wrap()
                                        .selectable(true),
                                );
                            }
                        }
                    });
            });
        },
    );
}

#[cfg(test)]
#[test]
fn wide_markdown_table_does_not_expand_chat_or_prevent_scrolling() {
    let ctx = egui::Context::default();
    setup(&ctx);
    let mut cache = egui_commonmark::CommonMarkCache::default();
    let mut files = files::FileViewer::default();
    let paragraph = "这段聊天正文包含一行连续的中文内容，用来检查窗口内的自动换行。".repeat(4);
    let cell = "a long table cell with many words ".repeat(12);
    let reply = format!("| Result | Details |\n| --- | --- |\n| OK | {cell} |\n\n{paragraph}");
    let work = Work::default();
    let cwd = Path::new("/tmp");
    let mut render = |events| {
        let mut scroll = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(960., 360.))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    scroll = Some(
                        egui::ScrollArea::vertical()
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                centered(ui, COLUMN, |ui| {
                                    ui.add_space(400.);
                                    for (index, text) in
                                        [&reply, &paragraph].into_iter().enumerate()
                                    {
                                        ui.push_id(("message", index), |ui| {
                                            render_reply(
                                                ui,
                                                &mut cache,
                                                &mut files,
                                                text,
                                                &work,
                                                ReplyFileLinks::Local(cwd),
                                            );
                                        });
                                    }
                                });
                            }),
                    );
                });
            },
        );
        (output, scroll.unwrap())
    };
    render(Vec::new());
    let (output, bottom) = render(Vec::new());
    assert!(bottom.state.offset.y > 0.);
    assert!(
        bottom.content_size.x <= bottom.inner_rect.width(),
        "chat exceeds the viewport: {:?}",
        bottom.content_size,
    );
    let mut paragraphs = 0;
    for clipped in &output.shapes {
        if let egui::Shape::Text(text) = &clipped.shape
            && text.galley.text() == paragraph
        {
            assert!(text.galley.rows.len() > 1);
            assert!(text.galley.size().x <= COLUMN + 1.);
            paragraphs += 1;
        }
    }
    assert_eq!(paragraphs, 2);

    let table_cell = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .find_map(|clipped| {
                if let egui::Shape::Text(text) = &clipped.shape
                    && text.galley.text() == cell.trim_end()
                {
                    Some((text.pos, clipped.clip_rect))
                } else {
                    None
                }
            })
            .expect("the long table cell should be rendered")
    };
    let (table_origin, table_clip) = table_cell(&output);
    let pointer = table_origin + vec2(15., 8.);
    assert!(table_clip.contains(pointer));
    assert!(table_clip.width() <= COLUMN + 2. * ctx.style().visuals.clip_rect_margin + 1.);
    let (_, scrolled) = render(vec![
        egui::Event::PointerMoved(pointer),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(-100., 0.),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    assert_eq!(scrolled.state.offset.y, bottom.state.offset.y);
    let (output, _) = render(Vec::new());
    assert!(table_cell(&output).0.x < table_origin.x);

    let (_, scrolled) = render(vec![
        egui::Event::PointerMoved(pointer),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0., 100.),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    assert!(scrolled.state.offset.y < bottom.state.offset.y);
}

#[cfg(test)]
#[test]
fn expanded_work_panel_renders_thinking_markdown() {
    let ctx = egui::Context::default();
    setup(&ctx);
    ctx.memory_mut(|memory| memory.set_everything_is_visible(true));
    let mut cache = egui_commonmark::CommonMarkCache::default();
    let mut files = files::FileViewer::default();
    let steps = [Step::thinking(
        "thought".into(),
        StepStatus::Complete,
        "**Checking files**",
    )];
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            render_work_steps(
                ui,
                &mut cache,
                &mut files,
                &steps,
                ReplyFileLinks::Local(Path::new("/tmp")),
            );
        });
    });
    let labels = output.shapes.iter().filter_map(|shape| {
        if let egui::Shape::Text(text) = &shape.shape {
            Some(text.galley.text())
        } else {
            None
        }
    });
    assert!(labels.clone().any(|text| text == "思考"));
    assert!(labels.clone().any(|text| text == "Checking files"));
    assert!(!labels.clone().any(|text| text.contains("**")));
}

fn disclosure(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    header: impl FnOnce(&mut Ui, f32) -> egui::Response,
    body: impl FnOnce(&mut Ui),
) {
    ui.push_id(id, |ui| {
        let id = ui.id();
        let mut state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        let openness = state.openness(ui.ctx());
        if header(ui, openness).clicked() {
            state.toggle(ui);
        }
        state.show_body_unindented(ui, body);
    });
}

fn disclosure_button(
    ui: &mut Ui,
    mut atoms: Atoms<'_>,
    width: f32,
    openness: f32,
    framed: bool,
) -> AtomLayoutResponse {
    let palette = Palette::of(ui);
    atoms.push_right(Icon::Right.atom());
    let button = ui
        .scope(|ui| {
            ui.spacing_mut().icon_spacing = 7.;
            Button::new(atoms)
                .truncate()
                .frame_when_inactive(framed)
                .min_size(vec2(width, 34.))
                .atom_ui(ui)
        })
        .inner;
    Icon::paint_chevron_in(
        ui,
        &button,
        openness,
        lerp_color(palette.muted, palette.accent, openness),
    );
    if button.response.has_focus() && !button.response.hovered() {
        ui.painter().rect_stroke(
            button.response.rect,
            8,
            Stroke::new(1., palette.accent),
            StrokeKind::Inside,
        );
    }
    if button.response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    button
}

fn turn_divider(ui: &mut Ui) {
    let palette = Palette::of(ui);
    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 1.));
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1., palette.line),
    );
}

fn change_counts(palette: Palette, added: usize, removed: usize) -> [RichText; 2] {
    [
        RichText::new(format!("+{added}"))
            .monospace()
            .size(12.)
            .color(palette.added),
        RichText::new(format!("−{removed}"))
            .monospace()
            .size(12.)
            .color(palette.danger),
    ]
}

fn render_changes(ui: &mut Ui, files: &[FileChange]) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    ui.add_space(9.);
    surface(palette, palette.paper, Margin::ZERO).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.;
        let added: usize = files.iter().map(|file| file.added).sum();
        let removed: usize = files.iter().map(|file| file.removed).sum();
        let [added, removed] = change_counts(palette, added, removed);
        let title = RichText::new(language.format(
            "已更改 {count} 个文件",
            &[("count", &files.len().to_string())],
        ))
        .strong();
        let summary = AtomLayout::new((Icon::Document.atom(), title, Atom::grow(), added, removed))
            .gap(7.)
            .frame(
                egui::Frame::new()
                    .fill(palette.soft)
                    .inner_margin(Margin::symmetric(12, 8))
                    .corner_radius(egui::CornerRadius {
                        nw: 11,
                        ne: 11,
                        ..Default::default()
                    }),
            )
            .min_size(vec2(ui.available_width(), 39.))
            .show(ui);
        Icon::Document.paint_in(ui, &summary, palette.accent);

        egui::Frame::new()
            .inner_margin(Margin::symmetric(2, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let (visible, more) = files.split_at(files.len().min(3));
                for (index, file) in visible.iter().enumerate() {
                    if index > 0 {
                        turn_divider(ui);
                    }
                    render_file_change(ui, file);
                }
                if !more.is_empty() {
                    turn_divider(ui);
                    disclosure(
                        ui,
                        "more files",
                        |ui, openness| {
                            let label = if openness > 0.5 {
                                language.text("收起其余文件").to_owned()
                            } else {
                                language.format(
                                    "另外 {count} 个文件",
                                    &[("count", &more.len().to_string())],
                                )
                            };
                            let atoms = Atoms::new((
                                RichText::new(label).size(12.5).color(palette.muted),
                                Atom::grow(),
                            ));
                            disclosure_button(ui, atoms, ui.available_width(), openness, false)
                                .response
                        },
                        |ui| {
                            for file in more {
                                turn_divider(ui);
                                render_file_change(ui, file);
                            }
                        },
                    );
                }
            });
    });
}

fn render_file_change(ui: &mut Ui, file: &FileChange) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let path = RichText::new(&file.path).monospace().size(13.);
    let [added, removed] = change_counts(palette, file.added, file.removed);
    let atoms = Atoms::new((Icon::Document.atom(), path, Atom::grow(), added, removed));
    if file.diff.is_empty() {
        let row = AtomLayout::new(atoms)
            .gap(7.)
            .frame(egui::Frame::new().inner_margin(Margin {
                right: 33,
                ..Margin::symmetric(10, 4)
            }))
            .wrap_mode(egui::TextWrapMode::Truncate)
            .min_size(vec2(ui.available_width(), 34.))
            .show(ui);
        Icon::Document.paint_in(ui, &row, palette.muted);
        row.response.on_hover_text(&file.path);
        return;
    }
    disclosure(
        ui,
        ("file change", &file.path),
        |ui, openness| {
            let button = disclosure_button(ui, atoms, ui.available_width(), openness, false);
            Icon::Document.paint_in(
                ui,
                &button,
                lerp_color(palette.muted, palette.accent, openness),
            );
            button.response.on_hover_text(&file.path)
        },
        |ui| {
            egui::Frame::new()
                .outer_margin(Margin {
                    left: 10,
                    right: 10,
                    top: 1,
                    bottom: 7,
                })
                .fill(palette.canvas)
                .stroke(Stroke::new(1., palette.line))
                .corner_radius(7)
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt(("diff", &file.path))
                        .max_height(220.)
                        .min_scrolled_height(220.)
                        .show(ui, |ui| {
                            let diff = super::syntax::highlight_diff_job(
                                ui.ctx(),
                                &file.diff,
                                &file.path,
                                12.5,
                                18.5,
                                ui.visuals().dark_mode,
                            );
                            ui.add(Label::new(diff).wrap().selectable(true));
                            if file.truncated {
                                ui.label(
                                    RichText::new(language.text("仅显示部分差异"))
                                        .size(12.)
                                        .color(palette.muted),
                                );
                            }
                        });
                });
        },
    );
}
