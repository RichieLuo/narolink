//! Desktop Git management right-hand split panel.
use super::{
    Language,
    git_diff::{DiffModalContext, DiffModalOutcome, DiffModalState, show_diff_modal},
    style::{Icon, Palette, solid, surface},
};
use crate::git::{
    GitAction, GitActionResult, GitDiff, GitDiffRequest, GitDiffScope, GitFileChange, GitStatus,
};
use eframe::egui::{
    self, Align, Button, Color32, FontId, Label, Layout, Margin, RichText, Stroke, TextEdit, Ui,
    vec2,
};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum GitTab {
    #[default]
    Changes,
    Branches,
    History,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum SelectedDiff {
    Staged(String),
    Unstaged(String),
    Untracked(String),
    Commit(String),
}

impl SelectedDiff {
    pub(super) fn to_request(&self, full_context: bool) -> GitDiffRequest {
        match self {
            Self::Staged(path) => GitDiffRequest {
                scope: GitDiffScope::Staged,
                path: Some(path.clone()),
                commit: None,
                full_context,
            },
            Self::Unstaged(path) => GitDiffRequest {
                scope: GitDiffScope::Unstaged,
                path: Some(path.clone()),
                commit: None,
                full_context,
            },
            Self::Untracked(path) => GitDiffRequest {
                scope: GitDiffScope::Untracked,
                path: Some(path.clone()),
                commit: None,
                full_context,
            },
            Self::Commit(hash) => GitDiffRequest {
                scope: GitDiffScope::Commit,
                path: None,
                commit: Some(hash.clone()),
                full_context: false,
            },
        }
    }
}

pub(super) enum GitCommand {
    Refresh,
    LoadDiff(GitDiffRequest),
    RunAction(GitAction),
    GenerateCommitMessage { amend: bool },
    Close,
}

#[derive(Default)]
pub(super) struct GitPanel {
    pub open: bool,
    pub cwd: Option<PathBuf>,
    pub tab: GitTab,
    pub status: Option<GitStatus>,
    pub loading: bool,
    pub action_pending: bool,
    pub generating_message: bool,
    pub serial: u64,
    pub diff_serial: u64,
    pub generate_serial: u64,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub commit_message: String,
    pub amend: bool,
    pub new_branch: String,
    pub selected_diff: Option<SelectedDiff>,
    pub diff: Option<GitDiff>,
    pub diff_loading: bool,
    pub diff_error: Option<String>,
    pub confirm_discard: Option<(String, bool)>,
    pub diff_modal: DiffModalState,
}

impl GitPanel {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.confirm_discard = None;
        self.diff_modal.close();
    }

    pub(super) fn reset_for_workspace(&mut self) {
        *self = Self::default();
    }

    pub(super) fn set_cwd(&mut self, cwd: Option<PathBuf>) -> bool {
        if self.cwd == cwd {
            return false;
        }
        self.cwd = cwd;
        self.status = None;
        self.error = None;
        self.notice = None;
        self.selected_diff = None;
        self.diff = None;
        self.diff_error = None;
        self.diff_loading = false;
        self.diff_modal.close();
        self.diff_modal.clear_diff();
        self.confirm_discard = None;
        self.generating_message = false;
        self.serial += 1;
        self.diff_serial += 1;
        self.generate_serial += 1;
        true
    }

    pub(super) fn apply_generated_message(&mut self, serial: u64, result: Result<String, String>) {
        if serial != self.generate_serial {
            return;
        }
        self.generating_message = false;
        match result {
            Ok(message) => {
                self.error = None;
                self.commit_message = message;
                self.notice = Some("已生成提交信息".to_owned());
            }
            Err(error) => {
                self.notice = None;
                self.error = Some(error);
            }
        }
    }

    pub(super) fn apply_status(&mut self, serial: u64, result: Result<GitStatus, String>) {
        if serial != self.serial {
            return;
        }
        self.loading = false;
        match result {
            Ok(status) => {
                self.error = None;
                self.prune_selected_diff(&status);
                self.status = Some(status);
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
    }

    pub(super) fn apply_diff(&mut self, serial: u64, result: Result<GitDiff, String>) {
        if serial != self.diff_serial {
            return;
        }
        self.diff_loading = false;
        match result {
            Ok(diff) => {
                self.diff_error = None;
                self.diff_modal.on_diff_loaded(serial, &diff);
                self.diff = Some(diff);
            }
            Err(error) => {
                self.diff = None;
                self.diff_modal.clear_diff();
                self.diff_error = Some(error);
            }
        }
    }

    pub(super) fn apply_action(
        &mut self,
        serial: u64,
        action: &GitAction,
        result: Result<GitActionResult, String>,
    ) -> Option<GitDiffRequest> {
        if serial != self.serial {
            return None;
        }
        self.action_pending = false;
        self.confirm_discard = None;
        match result {
            Ok(outcome) => {
                self.error = None;
                self.notice = Some(outcome.summary);
                if matches!(action, GitAction::Commit { .. }) {
                    self.commit_message.clear();
                    self.amend = false;
                }
                if matches!(action, GitAction::CreateBranch { .. }) {
                    self.new_branch.clear();
                }
                // Follow the file across staged/unstaged transitions while reviewing in diff view.
                self.follow_selected_after_action(action, &outcome.status);
                self.prune_selected_diff(&outcome.status);
                self.status = Some(outcome.status);
                let full_context = self.diff_modal.open;
                self.selected_diff
                    .as_ref()
                    .map(|sel| sel.to_request(full_context))
            }
            Err(error) => {
                self.notice = None;
                self.error = Some(error);
                None
            }
        }
    }

    fn follow_selected_after_action(&mut self, action: &GitAction, status: &GitStatus) {
        let Some(selected) = self.selected_diff.clone() else {
            return;
        };
        match (action, selected) {
            (
                GitAction::Stage { .. },
                SelectedDiff::Unstaged(path) | SelectedDiff::Untracked(path),
            ) if status.staged.iter().any(|item| item.path == path) => {
                self.selected_diff = Some(SelectedDiff::Staged(path));
            }
            (GitAction::Unstage { .. }, SelectedDiff::Staged(path)) => {
                if status.unstaged.iter().any(|item| item.path == path) {
                    self.selected_diff = Some(SelectedDiff::Unstaged(path));
                } else if status.untracked.iter().any(|item| item.path == path) {
                    self.selected_diff = Some(SelectedDiff::Untracked(path));
                }
            }
            _ => {}
        }
    }

    fn prune_selected_diff(&mut self, status: &GitStatus) {
        let keep = match &self.selected_diff {
            None => true,
            Some(SelectedDiff::Staged(path)) => status.staged.iter().any(|item| &item.path == path),
            Some(SelectedDiff::Unstaged(path)) => {
                status.unstaged.iter().any(|item| &item.path == path)
                    || status.conflicted.iter().any(|item| &item.path == path)
            }
            Some(SelectedDiff::Untracked(path)) => {
                status.untracked.iter().any(|item| &item.path == path)
            }
            Some(SelectedDiff::Commit(hash)) => {
                status.commits.iter().any(|item| &item.hash == hash)
            }
        };
        if !keep {
            self.selected_diff = None;
            self.diff = None;
            self.diff_error = None;
            self.diff_loading = false;
            self.diff_modal.close();
            self.diff_modal.clear_diff();
        }
    }

    fn open_diff_comparison(&mut self, selection: SelectedDiff) -> GitCommand {
        let same = self.selected_diff.as_ref() == Some(&selection);
        self.selected_diff = Some(selection.clone());
        if !same {
            self.diff = None;
            self.diff_error = None;
            self.diff_modal.clear_diff();
        }
        self.diff_modal
            .open_for_current(self.diff_serial, self.diff.as_ref());
        GitCommand::LoadDiff(selection.to_request(true))
    }

    pub(super) fn show_diff_modal(&mut self, ctx: &egui::Context) -> Option<GitCommand> {
        if !self.open || !self.diff_modal.open {
            return None;
        }
        let outcome = show_diff_modal(
            ctx,
            &mut self.diff_modal,
            DiffModalContext {
                selected: self.selected_diff.as_ref(),
                status: self.status.as_ref(),
                diff: self.diff.as_ref(),
                diff_serial: self.diff_serial,
                diff_loading: self.diff_loading,
                diff_error: self.diff_error.as_deref(),
                action_pending: self.action_pending,
            },
        );
        match outcome {
            DiffModalOutcome::None => None,
            DiffModalOutcome::SelectAndLoad(next) => Some(self.open_diff_comparison(next)),
            DiffModalOutcome::Command(cmd) => Some(cmd),
        }
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) -> Option<GitCommand> {
        if !self.open {
            return None;
        }
        let palette = Palette::from_context(ctx);
        let available = ctx.available_rect().width();
        let chat_width = (available * 0.46).clamp(360., 440.);
        let max_width = (available - chat_width).max(320.);
        let mut command = None;

        egui::SidePanel::right("git panel split")
            .default_width((available * 0.44).clamp(340., 540.))
            .width_range(320. ..=max_width)
            .resizable(true)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(palette.paper))
            .show(ctx, |ui| {
                // SidePanel persists its measured width. Keep overflowing content
                // from changing the width set by the resize handle.
                ui.take_available_width();
                let ui = &mut ui.new_child(egui::UiBuilder::new());
                let language = Language::in_ui(ui);
                let bounds = ui.max_rect();
                ui.spacing_mut().item_spacing.y = 0.;

                // Left hairline divider
                ui.painter().vline(
                    bounds.left() + 0.5,
                    bounds.y_range(),
                    Stroke::new(1., palette.line),
                );

                // Top header bar
                egui::Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.x = 4.;
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), 29.),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                if Icon::Close
                                    .button(ui, language.text("关闭 Git 面板"), palette.muted)
                                    .clicked()
                                {
                                    command = Some(GitCommand::Close);
                                }
                                let spinning = self.loading || self.action_pending;
                                if ui
                                    .add_enabled_ui(!self.action_pending, |ui| {
                                        Icon::refresh_button(
                                            ui,
                                            language.text("刷新 Git 状态"),
                                            palette.muted,
                                            spinning,
                                        )
                                    })
                                    .inner
                                    .clicked()
                                {
                                    command = Some(GitCommand::Refresh);
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 7.;
                                    let (rect, _) = ui
                                        .allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
                                    Icon::GitBranch.paint(
                                        ui.painter(),
                                        rect.center(),
                                        palette.accent,
                                    );
                                    let title = self
                                        .status
                                        .as_ref()
                                        .and_then(|st| st.repo_name.as_deref())
                                        .unwrap_or("Git");
                                    ui.add(
                                        Label::new(
                                            RichText::new(title)
                                                .size(13.5)
                                                .color(palette.ink)
                                                .strong(),
                                        )
                                        .truncate(),
                                    );
                                    if let Some(st) = &self.status
                                        && st.is_repo
                                    {
                                        let branch_text = if st.detached {
                                            format!(
                                                "HEAD {}",
                                                st.head_short.as_deref().unwrap_or("")
                                            )
                                        } else {
                                            st.branch.clone().unwrap_or_else(|| "HEAD".to_owned())
                                        };
                                        egui::Frame::new()
                                            .fill(palette.tint)
                                            .corner_radius(6)
                                            .inner_margin(Margin::symmetric(6, 2))
                                            .show(ui, |ui| {
                                                ui.add(
                                                    Label::new(
                                                        RichText::new(branch_text)
                                                            .size(11.5)
                                                            .color(palette.accent)
                                                            .strong(),
                                                    )
                                                    .truncate(),
                                                );
                                            });
                                        if st.ahead > 0 || st.behind > 0 {
                                            let sync = match (st.ahead, st.behind) {
                                                (a, 0) => format!("↑{a}"),
                                                (0, b) => format!("↓{b}"),
                                                (a, b) => format!("↑{a} ↓{b}"),
                                            };
                                            ui.label(
                                                RichText::new(sync).size(11.5).color(palette.muted),
                                            );
                                        }
                                    }
                                });
                            },
                        );
                    });

                // Sub-bar: tabs + sync actions
                let subbar = egui::Frame::new()
                    .fill(palette.paper)
                    .inner_margin(Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let is_repo = self.status.as_ref().is_some_and(|st| st.is_repo);
                        let has_remotes = self
                            .status
                            .as_ref()
                            .is_some_and(|st| !st.remotes.is_empty());
                        let changes_count =
                            self.status.as_ref().map_or(0, GitStatus::total_changes);
                        let branches_count = self.status.as_ref().map_or(0, |st| st.branches.len());

                        ui.spacing_mut().item_spacing = vec2(4., 4.);
                        egui::Sides::new().shrink_left().show(
                            ui,
                            |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().button_padding = vec2(8., 3.);
                                    for (tab, label, count) in [
                                        (GitTab::Changes, "变更", changes_count),
                                        (GitTab::Branches, "分支", branches_count),
                                        (GitTab::History, "历史", 0),
                                    ] {
                                        let label = language.text(label);
                                        let label = if count > 0 {
                                            format!("{label} ({count})")
                                        } else {
                                            label.to_owned()
                                        };
                                        let selected = self.tab == tab;
                                        let button = Button::new(
                                            RichText::new(label).size(12.).color(if selected {
                                                palette.accent
                                            } else {
                                                palette.muted
                                            }),
                                        )
                                        .small()
                                        .selected(selected)
                                        .frame_when_inactive(selected);
                                        if ui.add(button).clicked() {
                                            self.tab = tab;
                                        }
                                    }
                                });
                            },
                            |ui| {
                                if is_repo {
                                    ui.spacing_mut().button_padding = vec2(7., 3.);
                                    ui.add_enabled_ui(!self.action_pending && has_remotes, |ui| {
                                        for (label, action) in [
                                            ("推送", GitAction::Push),
                                            ("拉取", GitAction::Pull),
                                            ("获取", GitAction::Fetch),
                                        ] {
                                            let button = Button::new(
                                                RichText::new(language.text(label)).size(11.5),
                                            )
                                            .small();
                                            if ui.add(button).clicked() {
                                                command = Some(GitCommand::RunAction(action));
                                            }
                                        }
                                    });
                                }
                            },
                        );
                    });
                ui.painter().hline(
                    bounds.x_range(),
                    subbar.response.rect.bottom() - 0.5,
                    Stroke::new(1., palette.line),
                );

                // Banners: error / notice
                if let Some(error) = self.error.clone() {
                    egui::Frame::new()
                        .fill(palette.danger_surface)
                        .inner_margin(Margin::symmetric(12, 7))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if Icon::Close
                                        .button(ui, language.text("关闭"), palette.danger)
                                        .clicked()
                                    {
                                        self.error = None;
                                    }
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add(
                                            Label::new(
                                                RichText::new(error)
                                                    .size(12.)
                                                    .color(palette.danger),
                                            )
                                            .wrap(),
                                        );
                                    });
                                });
                            });
                        });
                } else if let Some(notice) = self.notice.clone() {
                    egui::Frame::new()
                        .fill(palette.notice)
                        .inner_margin(Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if Icon::Close
                                        .button(ui, language.text("关闭"), palette.muted)
                                        .clicked()
                                    {
                                        self.notice = None;
                                    }
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add(
                                            Label::new(
                                                RichText::new(language.text(&notice))
                                                    .size(12.)
                                                    .color(palette.accent),
                                            )
                                            .wrap(),
                                        );
                                    });
                                });
                            });
                        });
                }

                // Body
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.;
                        if self.cwd.is_none() {
                            ui.add_space(18.);
                            ui.label(
                                RichText::new(language.text("请先选择工作目录。"))
                                    .size(13.)
                                    .color(palette.muted),
                            );
                            return;
                        }
                        if self.status.is_none() {
                            ui.add_space(18.);
                            if self.loading {
                                ui.label(
                                    RichText::new(language.text("正在读取 Git 状态…"))
                                        .size(13.)
                                        .color(palette.muted),
                                );
                            } else {
                                ui.label(
                                    RichText::new(language.text("无法读取 Git 状态"))
                                        .size(13.)
                                        .color(palette.muted),
                                );
                            }
                            return;
                        }
                        let status = self.status.clone().unwrap();
                        if !status.is_repo {
                            ui.add_space(18.);
                            ui.label(
                                RichText::new(language.text("当前工作目录不是 Git 仓库。"))
                                    .size(13.)
                                    .color(palette.muted),
                            );
                            if let Some(cwd) = &self.cwd {
                                ui.label(
                                    RichText::new(cwd.to_string_lossy())
                                        .size(11.5)
                                        .color(palette.muted),
                                );
                            }
                            return;
                        }

                        egui::ScrollArea::vertical()
                            .id_salt(("git panel scroll", self.tab as u8))
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                match self.tab {
                                    GitTab::Changes => {
                                        if let Some(cmd) =
                                            self.show_changes(ui, palette, language, &status)
                                        {
                                            command = Some(cmd);
                                        }
                                    }
                                    GitTab::Branches => {
                                        if let Some(cmd) =
                                            self.show_branches(ui, palette, language, &status)
                                        {
                                            command = Some(cmd);
                                        }
                                    }
                                    GitTab::History => {
                                        if let Some(cmd) =
                                            self.show_history(ui, palette, language, &status)
                                        {
                                            command = Some(cmd);
                                        }
                                    }
                                }
                                if self.selected_diff.is_some()
                                    && matches!(self.tab, GitTab::Changes | GitTab::History)
                                {
                                    ui.add_space(8.);
                                    if let Some(cmd) = self.show_diff_viewer(ui, palette, language)
                                    {
                                        command = Some(cmd);
                                    }
                                }
                            });
                    });
            });

        // Cmd/Ctrl+D opens the Rider-style diff comparison modal for the selected item.
        if command.is_none()
            && !self.diff_modal.open
            && !ctx.wants_keyboard_input()
            && let Some(selected) = self.selected_diff.clone()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D))
        {
            command = Some(self.open_diff_comparison(selected));
        }

        command
    }

    fn show_changes(
        &mut self,
        ui: &mut Ui,
        palette: Palette,
        language: Language,
        status: &GitStatus,
    ) -> Option<GitCommand> {
        let mut command = None;

        // Commit box
        surface(palette, palette.canvas, Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.;
            let busy = self.action_pending || self.generating_message;
            ui.add_enabled(
                !busy,
                TextEdit::multiline(&mut self.commit_message)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(13.))
                    .hint_text(language.text("输入提交信息…")),
            );
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !busy && !status.commits.is_empty(),
                    egui::Checkbox::new(
                        &mut self.amend,
                        RichText::new(language.text("修正上次提交"))
                            .size(12.)
                            .color(palette.muted),
                    ),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.;
                    let can_commit = !busy
                        && (self.amend || !self.commit_message.trim().is_empty())
                        && (!status.staged.is_empty() || self.amend);
                    let fill = if can_commit {
                        palette.accent
                    } else {
                        palette.send_disabled
                    };
                    let hover = if can_commit {
                        palette.accent_hover
                    } else {
                        palette.send_disabled
                    };
                    let ink = if can_commit {
                        palette.on_accent
                    } else {
                        palette.send_disabled_text
                    };
                    let clicked = ui
                        .add_enabled_ui(can_commit, |ui| {
                            solid(ui, fill, hover, |ui| {
                                ui.add(
                                    Button::new(
                                        RichText::new(language.text("提交")).size(12.5).color(ink),
                                    )
                                    .min_size(vec2(64., 26.)),
                                )
                            })
                        })
                        .inner
                        .clicked();
                    if clicked {
                        command = Some(GitCommand::RunAction(GitAction::Commit {
                            message: self.commit_message.clone(),
                            amend: self.amend,
                        }));
                    }
                    let can_generate = !busy
                        && (status.total_changes() > 0
                            || (self.amend && !status.commits.is_empty()));
                    let gen_label = if self.generating_message {
                        language.text("生成中…")
                    } else {
                        language.text("AI 生成")
                    };
                    let gen_btn = ui
                        .add_enabled_ui(can_generate, |ui| {
                            let button = Icon::Lightning
                                .label(RichText::new(gen_label).size(12.).color(if can_generate {
                                    palette.accent
                                } else {
                                    palette.muted
                                }))
                                .min_size(vec2(0., 26.))
                                .atom_ui(ui);
                            Icon::Lightning.paint_in(
                                ui,
                                &button,
                                if can_generate {
                                    palette.accent
                                } else {
                                    palette.muted
                                },
                            );
                            button.response
                        })
                        .inner
                        .on_hover_text(language.text("使用 AI 生成提交信息"));
                    if gen_btn.clicked() {
                        command = Some(GitCommand::GenerateCommitMessage { amend: self.amend });
                    }
                });
            });
            if status.staged.is_empty() && !self.amend && status.total_changes() > 0 {
                ui.label(
                    RichText::new(language.text("暂存更改后再提交。"))
                        .size(11.5)
                        .color(palette.muted),
                );
            }
        });

        if status.total_changes() == 0 {
            ui.add_space(12.);
            ui.label(
                RichText::new(language.text("工作区干净，没有未提交更改。"))
                    .size(13.)
                    .color(palette.muted),
            );
            return command;
        }

        if status.truncated {
            ui.label(
                RichText::new(language.text("更改文件较多，仅显示部分条目。"))
                    .size(11.5)
                    .color(palette.warn),
            );
        }

        // Conflicted files
        if !status.conflicted.is_empty() {
            if let Some(cmd) = self.show_file_group(
                ui,
                palette,
                language,
                language.text("合并冲突"),
                &status.conflicted,
                FileGroupKind::Conflicted,
            ) {
                command = Some(cmd);
            }
        }

        // Staged files
        if !status.staged.is_empty() {
            if let Some(cmd) = self.show_file_group(
                ui,
                palette,
                language,
                language.text("已暂存"),
                &status.staged,
                FileGroupKind::Staged,
            ) {
                command = Some(cmd);
            }
        }

        // Unstaged files
        if !status.unstaged.is_empty() {
            if let Some(cmd) = self.show_file_group(
                ui,
                palette,
                language,
                language.text("更改"),
                &status.unstaged,
                FileGroupKind::Unstaged,
            ) {
                command = Some(cmd);
            }
        }

        // Untracked files
        if !status.untracked.is_empty() {
            if let Some(cmd) = self.show_file_group(
                ui,
                palette,
                language,
                language.text("未跟踪"),
                &status.untracked,
                FileGroupKind::Untracked,
            ) {
                command = Some(cmd);
            }
        }

        command
    }

    fn show_file_group(
        &mut self,
        ui: &mut Ui,
        palette: Palette,
        language: Language,
        title: &str,
        files: &[GitFileChange],
        kind: FileGroupKind,
    ) -> Option<GitCommand> {
        let mut command = None;
        ui.add_space(4.);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{title} ({})", files.len()))
                    .size(12.)
                    .color(palette.muted)
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(!self.action_pending, |ui| match kind {
                    FileGroupKind::Staged => {
                        if ui
                            .add(
                                Button::new(
                                    RichText::new(language.text("全部取消暂存"))
                                        .size(11.5)
                                        .color(palette.muted),
                                )
                                .small()
                                .frame_when_inactive(false),
                            )
                            .clicked()
                        {
                            command = Some(GitCommand::RunAction(GitAction::Unstage {
                                paths: Vec::new(),
                            }));
                        }
                    }
                    FileGroupKind::Unstaged | FileGroupKind::Untracked => {
                        if ui
                            .add(
                                Button::new(
                                    RichText::new(language.text("全部暂存"))
                                        .size(11.5)
                                        .color(palette.accent),
                                )
                                .small()
                                .frame_when_inactive(false),
                            )
                            .clicked()
                        {
                            let paths = files.iter().map(|f| f.path.clone()).collect();
                            command = Some(GitCommand::RunAction(GitAction::Stage { paths }));
                        }
                    }
                    FileGroupKind::Conflicted => {}
                });
            });
        });

        for file in files {
            let selection = match kind {
                FileGroupKind::Staged => SelectedDiff::Staged(file.path.clone()),
                FileGroupKind::Unstaged | FileGroupKind::Conflicted => {
                    SelectedDiff::Unstaged(file.path.clone())
                }
                FileGroupKind::Untracked => SelectedDiff::Untracked(file.path.clone()),
            };
            let is_selected = self.selected_diff.as_ref() == Some(&selection);
            let confirming = matches!(
                &self.confirm_discard,
                Some((path, untracked))
                    if path == &file.path && *untracked == (kind == FileGroupKind::Untracked)
            );
            let fill = if is_selected {
                palette.tint
            } else {
                Color32::TRANSPARENT
            };

            egui::Frame::new()
                .fill(fill)
                .corner_radius(7)
                .inner_margin(Margin::symmetric(6, 3))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // The whole row responds to click (preview) and double-click (Rider-style diff modal);
                    // inner action buttons keep priority.
                    let row = ui.scope_builder(
                        egui::UiBuilder::new().sense(egui::Sense::click()),
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 3.;
                                    ui.spacing_mut().button_padding = vec2(5., 2.);
                                    ui.add_enabled_ui(!self.action_pending, |ui| {
                                        if confirming {
                                            if ui
                                                .add(
                                                    Button::new(
                                                        RichText::new(language.text("取消"))
                                                            .size(11.)
                                                            .color(palette.muted),
                                                    )
                                                    .small(),
                                                )
                                                .clicked()
                                            {
                                                self.confirm_discard = None;
                                            }
                                            if ui
                                                .add(
                                                    Button::new(
                                                        RichText::new(language.text("确认丢弃"))
                                                            .size(11.)
                                                            .color(palette.danger),
                                                    )
                                                    .small(),
                                                )
                                                .clicked()
                                            {
                                                command = Some(GitCommand::RunAction(
                                                    GitAction::Discard {
                                                        paths: vec![file.path.clone()],
                                                        untracked: kind == FileGroupKind::Untracked,
                                                    },
                                                ));
                                            }
                                        } else {
                                            match kind {
                                                FileGroupKind::Staged => {
                                                    if ui
                                                        .add(
                                                            Button::new(
                                                                RichText::new("−")
                                                                    .size(13.)
                                                                    .color(palette.muted),
                                                            )
                                                            .small()
                                                            .frame_when_inactive(false),
                                                        )
                                                        .on_hover_text(language.text("取消暂存"))
                                                        .clicked()
                                                    {
                                                        command = Some(GitCommand::RunAction(
                                                            GitAction::Unstage {
                                                                paths: vec![file.path.clone()],
                                                            },
                                                        ));
                                                    }
                                                }
                                                FileGroupKind::Unstaged
                                                | FileGroupKind::Untracked => {
                                                    if ui
                                                        .add(
                                                            Button::new(
                                                                RichText::new("↩")
                                                                    .size(12.)
                                                                    .color(palette.danger),
                                                            )
                                                            .small()
                                                            .frame_when_inactive(false),
                                                        )
                                                        .on_hover_text(language.text("丢弃更改"))
                                                        .clicked()
                                                    {
                                                        self.confirm_discard = Some((
                                                            file.path.clone(),
                                                            kind == FileGroupKind::Untracked,
                                                        ));
                                                    }
                                                    if ui
                                                        .add(
                                                            Button::new(
                                                                RichText::new("+")
                                                                    .size(13.)
                                                                    .color(palette.added),
                                                            )
                                                            .small()
                                                            .frame_when_inactive(false),
                                                        )
                                                        .on_hover_text(language.text("暂存"))
                                                        .clicked()
                                                    {
                                                        command = Some(GitCommand::RunAction(
                                                            GitAction::Stage {
                                                                paths: vec![file.path.clone()],
                                                            },
                                                        ));
                                                    }
                                                }
                                                FileGroupKind::Conflicted => {
                                                    if ui
                                                        .add(
                                                            Button::new(
                                                                RichText::new("+")
                                                                    .size(13.)
                                                                    .color(palette.added),
                                                            )
                                                            .small()
                                                            .frame_when_inactive(false),
                                                        )
                                                        .on_hover_text(language.text("暂存"))
                                                        .clicked()
                                                    {
                                                        command = Some(GitCommand::RunAction(
                                                            GitAction::Stage {
                                                                paths: vec![file.path.clone()],
                                                            },
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    });

                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        let badge_color = status_color(&file.status, palette);
                                        ui.add(
                                            Label::new(
                                                RichText::new(&file.status)
                                                    .size(11.5)
                                                    .monospace()
                                                    .color(badge_color)
                                                    .strong(),
                                            )
                                            .selectable(false),
                                        );
                                        let display = match &file.orig_path {
                                            Some(orig) => format!("{orig} → {}", file.path),
                                            None => file.path.clone(),
                                        };
                                        ui.add(
                                            Label::new(
                                                RichText::new(&display)
                                                    .size(12.5)
                                                    .color(palette.ink),
                                            )
                                            .truncate()
                                            .selectable(false),
                                        );
                                    });
                                });
                            });
                        },
                    );
                    let tip = match &file.orig_path {
                        Some(orig) => format!(
                            "{orig} → {}\n{}",
                            file.path,
                            language.text("双击打开并排差异对比")
                        ),
                        None => format!("{}\n{}", file.path, language.text("双击打开并排差异对比")),
                    };
                    let row = row
                        .response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(tip);
                    if row.double_clicked() {
                        command = Some(self.open_diff_comparison(selection));
                    } else if row.clicked() {
                        let already_loaded = self.selected_diff.as_ref() == Some(&selection)
                            && (self.diff.is_some() || self.diff_loading);
                        self.selected_diff = Some(selection.clone());
                        if !already_loaded {
                            command = Some(GitCommand::LoadDiff(selection.to_request(false)));
                        }
                    }
                });
        }
        command
    }

    fn show_branches(
        &mut self,
        ui: &mut Ui,
        palette: Palette,
        language: Language,
        status: &GitStatus,
    ) -> Option<GitCommand> {
        let mut command = None;

        // Create branch box
        surface(palette, palette.canvas, Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.;
            ui.label(
                RichText::new(language.text("新建分支"))
                    .size(12.)
                    .color(palette.muted)
                    .strong(),
            );
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), ui.spacing().interact_size.y),
                Layout::right_to_left(Align::Center),
                |ui| {
                    let can_create = !self.action_pending && !self.new_branch.trim().is_empty();
                    let clicked = ui
                        .add_enabled(
                            can_create,
                            Button::new(RichText::new(language.text("创建并切换")).size(12.))
                                .small(),
                        )
                        .clicked();
                    let resp = ui.add_enabled(
                        !self.action_pending,
                        TextEdit::singleline(&mut self.new_branch)
                            .desired_width(f32::INFINITY)
                            .hint_text(language.text("输入新分支名称")),
                    );
                    let enter =
                        resp.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let name = self.new_branch.trim();
                    if (clicked || enter) && !self.action_pending && !name.is_empty() {
                        command = Some(GitCommand::RunAction(GitAction::CreateBranch {
                            name: name.to_owned(),
                            checkout: true,
                        }));
                    }
                },
            );
        });

        ui.add_space(6.);
        ui.label(
            RichText::new(format!(
                "{} ({})",
                language.text("本地分支"),
                status.branches.len()
            ))
            .size(12.)
            .color(palette.muted)
            .strong(),
        );

        if status.branches.is_empty() {
            ui.label(
                RichText::new(language.text("暂无分支。"))
                    .size(12.5)
                    .color(palette.muted),
            );
            return command;
        }

        for branch in &status.branches {
            let fill = if branch.current {
                palette.tint
            } else {
                palette.canvas
            };
            egui::Frame::new()
                .fill(fill)
                .stroke(Stroke::new(
                    1.,
                    if branch.current {
                        palette.tint_line
                    } else {
                        palette.line
                    },
                ))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if branch.current {
                                ui.label(
                                    RichText::new(language.text("当前分支"))
                                        .size(11.5)
                                        .color(palette.accent)
                                        .strong(),
                                );
                            } else if ui
                                .add_enabled(
                                    !self.action_pending,
                                    Button::new(RichText::new(language.text("切换")).size(11.5))
                                        .small(),
                                )
                                .clicked()
                            {
                                command = Some(GitCommand::RunAction(GitAction::Checkout {
                                    branch: branch.name.clone(),
                                }));
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(
                                    Label::new(
                                        RichText::new(&branch.name)
                                            .size(13.)
                                            .color(palette.ink)
                                            .strong(),
                                    )
                                    .truncate(),
                                );
                                if let Some(short) = &branch.commit_short {
                                    ui.add(
                                        Label::new(
                                            RichText::new(short)
                                                .size(11.)
                                                .monospace()
                                                .color(palette.muted),
                                        )
                                        .truncate(),
                                    );
                                }
                            });
                        });
                    });
                    if !branch.subject.is_empty() || branch.upstream.is_some() {
                        ui.horizontal(|ui| {
                            if let Some(upstream) = &branch.upstream {
                                let mut meta = upstream.clone();
                                if branch.ahead > 0 {
                                    meta.push_str(&format!(" ↑{}", branch.ahead));
                                }
                                if branch.behind > 0 {
                                    meta.push_str(&format!(" ↓{}", branch.behind));
                                }
                                ui.add(
                                    Label::new(RichText::new(meta).size(11.).color(palette.accent))
                                        .truncate(),
                                );
                            }
                            if !branch.subject.is_empty() {
                                ui.add(
                                    Label::new(
                                        RichText::new(&branch.subject)
                                            .size(11.5)
                                            .color(palette.muted),
                                    )
                                    .truncate(),
                                );
                            }
                        });
                    }
                });
        }

        command
    }

    fn show_history(
        &mut self,
        ui: &mut Ui,
        palette: Palette,
        language: Language,
        status: &GitStatus,
    ) -> Option<GitCommand> {
        let mut command = None;
        if status.commits.is_empty() {
            ui.add_space(12.);
            ui.label(
                RichText::new(language.text("暂无提交记录。"))
                    .size(13.)
                    .color(palette.muted),
            );
            return None;
        }
        for commit in &status.commits {
            let selection = SelectedDiff::Commit(commit.hash.clone());
            let selected = self.selected_diff.as_ref() == Some(&selection);
            let fill = if selected {
                palette.tint
            } else {
                palette.canvas
            };
            let frame = egui::Frame::new()
                .fill(fill)
                .stroke(Stroke::new(
                    1.,
                    if selected {
                        palette.tint_line
                    } else {
                        palette.line
                    },
                ))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        Label::new(
                            RichText::new(&commit.subject)
                                .size(12.5)
                                .color(palette.ink)
                                .strong(),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            Label::new(
                                RichText::new(&commit.short_hash)
                                    .size(11.)
                                    .monospace()
                                    .color(palette.accent),
                            )
                            .selectable(false),
                        );
                        ui.add(
                            Label::new(
                                RichText::new(&commit.author)
                                    .size(11.5)
                                    .color(palette.muted),
                            )
                            .selectable(false),
                        );
                        ui.add(
                            Label::new(
                                RichText::new(&commit.relative_date)
                                    .size(11.5)
                                    .color(palette.muted),
                            )
                            .selectable(false),
                        );
                    });
                });
            let row = ui
                .interact(
                    frame.response.rect,
                    ui.id().with(("git commit row", &commit.hash)),
                    egui::Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(language.text("双击打开并排差异对比"));
            if row.double_clicked() {
                command = Some(self.open_diff_comparison(selection));
            } else if row.clicked() {
                let already_loaded = self.selected_diff.as_ref() == Some(&selection)
                    && (self.diff.is_some() || self.diff_loading);
                self.selected_diff = Some(selection.clone());
                if !already_loaded {
                    command = Some(GitCommand::LoadDiff(selection.to_request(false)));
                }
            }
        }
        command
    }

    fn show_diff_viewer(
        &mut self,
        ui: &mut Ui,
        palette: Palette,
        language: Language,
    ) -> Option<GitCommand> {
        let mut command = None;
        surface(palette, palette.canvas, Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let title = self
                    .diff
                    .as_ref()
                    .map(|diff| diff.title.clone())
                    .or_else(|| match &self.selected_diff {
                        Some(SelectedDiff::Staged(p))
                        | Some(SelectedDiff::Unstaged(p))
                        | Some(SelectedDiff::Untracked(p)) => Some(p.clone()),
                        Some(SelectedDiff::Commit(h)) => Some(h.clone()),
                        None => None,
                    })
                    .unwrap_or_default();
                let mut close_diff = false;
                let mut open_modal = false;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if Icon::Close
                        .button(ui, language.text("关闭差异"), palette.muted)
                        .clicked()
                    {
                        close_diff = true;
                    }
                    if ui
                        .add(
                            Button::new(
                                RichText::new(language.text("并排对比"))
                                    .size(11.)
                                    .color(palette.accent),
                            )
                            .small(),
                        )
                        .on_hover_text(language.text("双击文件或在弹窗中并排对比差异"))
                        .clicked()
                    {
                        open_modal = true;
                    }
                    if let Some(diff) = &self.diff {
                        if diff.removed > 0 {
                            ui.label(
                                RichText::new(format!("-{}", diff.removed))
                                    .size(11.5)
                                    .monospace()
                                    .color(palette.danger),
                            );
                        }
                        if diff.added > 0 {
                            ui.label(
                                RichText::new(format!("+{}", diff.added))
                                    .size(11.5)
                                    .monospace()
                                    .color(palette.added),
                            );
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(
                            Label::new(
                                RichText::new(&title)
                                    .size(12.)
                                    .monospace()
                                    .color(palette.ink)
                                    .strong(),
                            )
                            .truncate(),
                        );
                    });
                });
                if open_modal && let Some(selected) = self.selected_diff.clone() {
                    command = Some(self.open_diff_comparison(selected));
                }
                if close_diff {
                    self.selected_diff = None;
                    self.diff = None;
                    self.diff_error = None;
                    self.diff_loading = false;
                    self.diff_modal.close();
                    self.diff_modal.clear_diff();
                }
            });
            ui.separator();
            if self.diff_loading {
                ui.label(
                    RichText::new(language.text("正在加载差异…"))
                        .size(12.)
                        .color(palette.muted),
                );
                return;
            }
            if let Some(error) = &self.diff_error {
                ui.label(RichText::new(error).size(12.).color(palette.danger));
                return;
            }
            let Some(diff) = &self.diff else {
                return;
            };
            if diff.binary {
                ui.label(
                    RichText::new(language.text("二进制文件，无法显示文本差异。"))
                        .size(12.)
                        .color(palette.muted),
                );
                return;
            }
            if diff.diff.is_empty() {
                ui.label(
                    RichText::new(language.text("没有文本差异。"))
                        .size(12.)
                        .color(palette.muted),
                );
                return;
            }
            egui::ScrollArea::both()
                .id_salt(("git diff scroll", &diff.title))
                .max_height(320.)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let mut job = super::syntax::highlight_diff_job(
                        ui.ctx(),
                        &diff.diff,
                        &diff.title,
                        12.,
                        18.,
                        ui.visuals().dark_mode,
                    );
                    if diff.truncated {
                        job.append(
                            &format!("\n… {}", language.text("仅显示部分差异")),
                            0.,
                            egui::TextFormat::simple(FontId::monospace(12.), palette.muted),
                        );
                    }
                    ui.add(Label::new(job).selectable(true));
                });
        });
        command
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileGroupKind {
    Conflicted,
    Staged,
    Unstaged,
    Untracked,
}

fn status_color(status: &str, palette: Palette) -> Color32 {
    match status {
        "A" | "?" => palette.added,
        "D" => palette.danger,
        "M" | "R" | "C" => palette.accent,
        _ => palette.warn,
    }
}

#[cfg(test)]
#[test]
fn git_panel_keeps_its_width_after_switching_tabs_and_dragging() {
    use crate::git::GitBranch;
    use egui::containers::panel::PanelState;
    use egui::{Event, Id, Modifiers, PointerButton, Pos2, Rect, pos2};

    for language in Language::ALL {
        let ctx = egui::Context::default();
        super::style::setup(&ctx);
        language.apply(&ctx);
        let status = GitStatus {
            is_repo: true,
            repo_name: Some("naro-link".into()),
            branch: Some("main".into()),
            remotes: vec!["origin".into()],
            unstaged: vec![GitFileChange {
                path: "src/desktop/git.rs".into(),
                orig_path: None,
                status: "M".into(),
            }],
            branches: vec![GitBranch {
                name: "main".into(),
                current: true,
                upstream: Some("origin/main".into()),
                ahead: 0,
                behind: 0,
                commit_short: Some("abc1234".into()),
                subject: "Update the workspace panel".into(),
            }],
            ..Default::default()
        };
        let mut panel = GitPanel {
            open: true,
            cwd: Some("/tmp/naro-link".into()),
            status: Some(status),
            ..Default::default()
        };
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000., 700.));
        let render = |panel: &mut GitPanel, events| {
            let mut command = None;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| command = panel.show(ctx),
            );
            (output, command)
        };
        let panel_rect = || {
            PanelState::load(&ctx, Id::new("git panel split"))
                .unwrap()
                .rect
        };
        let assert_width = |expected: f32| {
            let actual = panel_rect().width();
            assert!(
                (actual - expected).abs() <= 0.5,
                "{}: expected width {expected}, got {actual}",
                language.code(),
            );
        };
        let text_center = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        let rect = text.visual_bounding_rect().intersect(clipped.clip_rect);
                        rect.is_positive().then_some(rect.center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{}: missing visible text {label}", language.code()))
        };
        let pointer_button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let click = |pos| {
            vec![
                Event::PointerMoved(pos),
                pointer_button(pos, true),
                pointer_button(pos, false),
            ]
        };

        let (changes, _) = render(&mut panel, vec![]);
        let initial = panel_rect();
        let branches_tab = format!("{} (1)", language.text("分支"));
        render(&mut panel, click(text_center(&changes, &branches_tab)));
        assert!(panel.tab == GitTab::Branches);
        assert_width(initial.width());

        let start = initial.left_center();
        let end = pos2(screen.right() - 320., start.y);
        render(&mut panel, vec![Event::PointerMoved(start)]);
        render(&mut panel, vec![pointer_button(start, true)]);
        render(&mut panel, vec![Event::PointerMoved(end)]);
        render(&mut panel, vec![pointer_button(end, false)]);
        assert_width(320.);

        // A status refresh can introduce much longer branch and upstream names.
        let branch = &mut panel.status.as_mut().unwrap().branches[0];
        branch.name = format!("feature/{}", "long-name-".repeat(12));
        branch.upstream = Some(format!("origin/{}", branch.name));
        for _ in 0..3 {
            render(&mut panel, vec![]);
            assert_width(320.);
        }

        let (branches, _) = render(&mut panel, vec![]);
        let branch_input = text_center(&branches, language.text("输入新分支名称"));
        render(&mut panel, click(branch_input));
        render(&mut panel, vec![Event::Text("example-branch".into())]);
        assert_eq!(panel.new_branch, "example-branch");
        let (branches, _) = render(&mut panel, vec![]);
        let create_button = text_center(&branches, language.text("创建并切换"));
        let (_, command) = render(&mut panel, click(create_button));
        assert!(matches!(
            command,
            Some(GitCommand::RunAction(GitAction::CreateBranch { name, checkout: true }))
                if name == "example-branch"
        ));

        let (branches, _) = render(&mut panel, vec![]);
        render(
            &mut panel,
            click(text_center(&branches, language.text("历史"))),
        );
        assert!(
            panel.tab == GitTab::History,
            "{}: History tab was not selected",
            language.code()
        );
        assert_width(320.);
    }
}

#[cfg(test)]
#[test]
fn double_clicking_file_opens_side_by_side_diff_modal() {
    use super::git_diff::DiffViewMode;
    use egui::{Event, Modifiers, PointerButton, Pos2, Rect};

    let ctx = egui::Context::default();
    super::style::setup(&ctx);
    Language::Chinese.apply(&ctx);
    let status = GitStatus {
        is_repo: true,
        repo_name: Some("naro-link".into()),
        branch: Some("main".into()),
        unstaged: vec![
            GitFileChange {
                path: "src/desktop/git.rs".into(),
                orig_path: None,
                status: "M".into(),
            },
            GitFileChange {
                path: "src/desktop/git_diff.rs".into(),
                orig_path: None,
                status: "M".into(),
            },
        ],
        ..Default::default()
    };
    let mut panel = GitPanel {
        open: true,
        cwd: Some("/tmp/naro-link".into()),
        status: Some(status),
        ..Default::default()
    };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1200., 800.));
    let mut time = 0.0_f64;
    let mut render = |panel: &mut GitPanel, events: Vec<Event>, dt: f64| {
        time += dt;
        let mut command = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                if let Some(cmd) = panel.show(ctx) {
                    command = Some(cmd);
                }
                if let Some(cmd) = panel.show_diff_modal(ctx) {
                    command = Some(cmd);
                }
            },
        );
        (output, command)
    };
    let text_center = |output: &egui::FullOutput, label: &str| {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    let rect = text.visual_bounding_rect().intersect(clipped.clip_rect);
                    rect.is_positive().then_some(rect.center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing visible text {label}"))
    };
    let click_events = |pos| {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ]
    };

    let (initial, _) = render(&mut panel, vec![], 0.016);
    let file_pos = text_center(&initial, "src/desktop/git.rs");

    // First click selects the file for inline preview without opening the modal.
    let (_, first_cmd) = render(&mut panel, click_events(file_pos), 0.05);
    assert!(matches!(
        first_cmd,
        Some(GitCommand::LoadDiff(ref req)) if !req.full_context && req.path.as_deref() == Some("src/desktop/git.rs")
    ));
    assert!(!panel.diff_modal.open);

    // Second click within the double-click window opens the Rider-style comparison modal.
    let (_, second_cmd) = render(&mut panel, click_events(file_pos), 0.05);
    assert!(matches!(
        second_cmd,
        Some(GitCommand::LoadDiff(ref req)) if req.full_context && req.path.as_deref() == Some("src/desktop/git.rs")
    ));
    assert!(panel.diff_modal.open);
    assert!(panel.diff_modal.mode == DiffViewMode::SideBySide);

    // Load diff content and render the modal in both SideBySide and Unified modes.
    panel.diff_serial = 1;
    panel.apply_diff(
        1,
        Ok(GitDiff {
            title: "src/desktop/git.rs".into(),
            diff: "\
diff --git a/src/desktop/git.rs b/src/desktop/git.rs
--- a/src/desktop/git.rs
+++ b/src/desktop/git.rs
@@ -1,3 +1,4 @@
 fn a() {}
-let x = 1;
+let x = 2;
+let y = 3;
 fn b() {}
"
            .into(),
            added: 2,
            removed: 1,
            truncated: false,
            binary: false,
        }),
    );
    let (modal_out, _) = render(&mut panel, vec![], 0.05);
    let unified_btn = text_center(&modal_out, "统一视图");
    render(&mut panel, click_events(unified_btn), 0.05);
    assert!(panel.diff_modal.mode == DiffViewMode::Unified);

    // Pressing Escape closes the comparison modal.
    render(
        &mut panel,
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        0.05,
    );
    assert!(!panel.diff_modal.open);
}
