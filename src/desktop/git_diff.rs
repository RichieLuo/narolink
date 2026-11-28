//! JetBrains Rider-inspired diff comparison modal for the desktop Git panel.
use super::{
    Language,
    git::{GitCommand, SelectedDiff},
    style::{Icon, Palette, popup},
    syntax as desktop_syntax,
};
use crate::{
    git::{GitAction, GitDiff, GitStatus},
    syntax as core_syntax,
};
use eframe::egui::{
    self, Align, Align2, Button, Color32, FontId, Id, Key, Label, Layout, Margin, Rect, RichText,
    Sense, Stroke, Ui, pos2, vec2,
};
use std::{collections::HashSet, ops::Range, path::Path};

const CODE_FONT_SIZE: f32 = 12.5;
const ROW_HEIGHT: f32 = 20.0;
const HEADER_ROW_HEIGHT: f32 = 25.0;
const GUTTER_MIN_WIDTH: f32 = 44.0;
const DIVIDER_BAND_WIDTH: f32 = 8.0;
const OVERVIEW_WIDTH: f32 = 14.0;
const FOLD_KEEP_CONTEXT: usize = 3;
const FOLD_MIN_SKIP: usize = 6;
const MAX_INLINE_DIFF_CELLS: usize = 2400;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum DiffViewMode {
    #[default]
    SideBySide,
    Unified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PairKind {
    Context,
    Added,
    Removed,
    Modified,
}

#[derive(Clone, Debug)]
enum DiffRow {
    FileHeader {
        path: String,
    },
    HunkHeader {
        header: String,
    },
    CommitMeta {
        text: String,
    },
    Pair {
        lang: String,
        left_line: Option<usize>,
        right_line: Option<usize>,
        left_text: String,
        right_text: String,
        left_emph: Vec<Range<usize>>,
        right_emph: Vec<Range<usize>>,
        kind: PairKind,
    },
}

#[derive(Clone, Copy, Debug)]
struct ChangeBlock {
    start_row: usize,
    end_row: usize, // exclusive
    kind: PairKind,
}

struct ParsedDiff {
    rows: Vec<DiffRow>,
    change_blocks: Vec<ChangeBlock>,
    max_line_num: usize,
    has_collapsible_context: bool,
    multi_file: bool,
}

enum DisplayItem {
    Row(usize),
    Folded {
        fold_id: usize,
        skipped_lines: usize,
    },
}

enum UnifiedDisplayItem {
    FileHeader {
        path: String,
    },
    HunkHeader {
        header: String,
    },
    CommitMeta {
        text: String,
    },
    Folded {
        fold_id: usize,
        skipped_lines: usize,
    },
    Line {
        source_row: usize,
        lang: String,
        old_line: Option<usize>,
        new_line: Option<usize>,
        prefix: char,
        text: String,
        emph: Vec<Range<usize>>,
        kind: PairKind,
    },
}

#[derive(Clone, Copy)]
struct DiffColors {
    added_bg: Color32,
    added_emph: Color32,
    added_gutter: Color32,
    added_bar: Color32,
    removed_bg: Color32,
    removed_emph: Color32,
    removed_gutter: Color32,
    removed_bar: Color32,
    modified_bar: Color32,
    empty_slot: Color32,
    hunk_bg: Color32,
    file_header_bg: Color32,
    fold_bg: Color32,
    selected_outline: Color32,
}

impl DiffColors {
    fn of(dark_mode: bool) -> Self {
        if dark_mode {
            Self {
                added_bg: Color32::from_rgb(26, 52, 39),
                added_emph: Color32::from_rgb(44, 96, 66),
                added_gutter: Color32::from_rgb(31, 63, 47),
                added_bar: Color32::from_rgb(86, 196, 134),
                removed_bg: Color32::from_rgb(62, 34, 43),
                removed_emph: Color32::from_rgb(112, 49, 64),
                removed_gutter: Color32::from_rgb(74, 39, 50),
                removed_bar: Color32::from_rgb(232, 108, 124),
                modified_bar: Color32::from_rgb(112, 168, 238),
                empty_slot: Color32::from_rgb(27, 31, 39),
                hunk_bg: Color32::from_rgb(31, 41, 56),
                file_header_bg: Color32::from_rgb(30, 36, 46),
                fold_bg: Color32::from_rgb(28, 35, 46),
                selected_outline: Color32::from_rgb(130, 183, 245),
            }
        } else {
            Self {
                added_bg: Color32::from_rgb(231, 246, 235),
                added_emph: Color32::from_rgb(176, 230, 191),
                added_gutter: Color32::from_rgb(212, 239, 220),
                added_bar: Color32::from_rgb(50, 158, 104),
                removed_bg: Color32::from_rgb(252, 235, 238),
                removed_emph: Color32::from_rgb(246, 188, 197),
                removed_gutter: Color32::from_rgb(247, 217, 222),
                removed_bar: Color32::from_rgb(206, 76, 92),
                modified_bar: Color32::from_rgb(66, 133, 212),
                empty_slot: Color32::from_rgb(244, 247, 251),
                hunk_bg: Color32::from_rgb(235, 242, 251),
                file_header_bg: Color32::from_rgb(238, 243, 249),
                fold_bg: Color32::from_rgb(240, 245, 251),
                selected_outline: Color32::from_rgb(53, 112, 181),
            }
        }
    }
}

#[derive(Default)]
pub(super) struct DiffModalState {
    pub open: bool,
    pub mode: DiffViewMode,
    pub collapse_unchanged: bool,
    expanded_folds: HashSet<usize>,
    pub current_change: usize,
    scroll_to_source_row: Option<usize>,
    h_scroll: f32,
    content_max_width: f32,
    selected_row: Option<usize>,
    cached_key: Option<(u64, String)>,
    parsed: Option<ParsedDiff>,
}

impl DiffModalState {
    pub(super) fn open_for_current(&mut self, serial: u64, diff: Option<&GitDiff>) {
        self.open = true;
        self.h_scroll = 0.0;
        self.selected_row = None;
        self.expanded_folds.clear();
        self.current_change = 0;
        if let Some(diff) = diff {
            self.ensure_parsed(serial, diff);
            if let Some(parsed) = &self.parsed {
                self.scroll_to_source_row = parsed.change_blocks.first().map(|b| b.start_row);
            }
        } else {
            self.scroll_to_source_row = None;
        }
    }

    pub(super) fn on_diff_loaded(&mut self, serial: u64, diff: &GitDiff) {
        self.h_scroll = 0.0;
        self.selected_row = None;
        self.expanded_folds.clear();
        self.current_change = 0;
        self.ensure_parsed(serial, diff);
        if self.open {
            self.scroll_to_source_row = self
                .parsed
                .as_ref()
                .and_then(|p| p.change_blocks.first().map(|b| b.start_row));
        }
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.scroll_to_source_row = None;
    }

    pub(super) fn clear_diff(&mut self) {
        self.cached_key = None;
        self.parsed = None;
        self.scroll_to_source_row = None;
        self.selected_row = None;
        self.current_change = 0;
        self.h_scroll = 0.0;
    }

    fn ensure_parsed(&mut self, serial: u64, diff: &GitDiff) {
        let key = (serial, diff.title.clone());
        if self.cached_key.as_ref() == Some(&key) && self.parsed.is_some() {
            return;
        }
        self.parsed = Some(parse_unified_diff(&diff.diff, &diff.title));
        self.cached_key = Some(key);
        self.content_max_width = 0.0;
    }

    fn jump_change(&mut self, delta: isize) {
        let Some(parsed) = &self.parsed else {
            return;
        };
        let count = parsed.change_blocks.len();
        if count == 0 {
            return;
        }
        let next = if delta >= 0 {
            (self.current_change + delta as usize) % count
        } else {
            let step = (-delta) as usize % count;
            (self.current_change + count - step) % count
        };
        self.current_change = next;
        self.scroll_to_source_row = Some(parsed.change_blocks[next].start_row);
    }
}

pub(super) struct DiffModalContext<'a> {
    pub selected: Option<&'a SelectedDiff>,
    pub status: Option<&'a GitStatus>,
    pub diff: Option<&'a GitDiff>,
    pub diff_serial: u64,
    pub diff_loading: bool,
    pub diff_error: Option<&'a str>,
    pub action_pending: bool,
}

pub(super) enum DiffModalOutcome {
    None,
    SelectAndLoad(SelectedDiff),
    Command(GitCommand),
}

/// Build the navigable sequence of entries matching the currently open diff kind.
fn build_nav_list(
    selected: &SelectedDiff,
    status: &GitStatus,
) -> (Vec<SelectedDiff>, Option<usize>) {
    let items: Vec<SelectedDiff> = match selected {
        SelectedDiff::Commit(_) => status
            .commits
            .iter()
            .map(|c| SelectedDiff::Commit(c.hash.clone()))
            .collect(),
        _ => {
            let mut list = Vec::new();
            for f in &status.conflicted {
                list.push(SelectedDiff::Unstaged(f.path.clone()));
            }
            for f in &status.staged {
                list.push(SelectedDiff::Staged(f.path.clone()));
            }
            for f in &status.unstaged {
                list.push(SelectedDiff::Unstaged(f.path.clone()));
            }
            for f in &status.untracked {
                list.push(SelectedDiff::Untracked(f.path.clone()));
            }
            list
        }
    };
    let index = items.iter().position(|item| item == selected);
    (items, index)
}

pub(super) fn show_diff_modal(
    ctx: &egui::Context,
    state: &mut DiffModalState,
    info: DiffModalContext<'_>,
) -> DiffModalOutcome {
    if !state.open {
        return DiffModalOutcome::None;
    }
    let Some(selected) = info.selected.cloned() else {
        state.close();
        return DiffModalOutcome::None;
    };
    if let Some(diff) = info.diff {
        state.ensure_parsed(info.diff_serial, diff);
    }

    let palette = Palette::from_context(ctx);
    let dark_mode = ctx.theme() == egui::Theme::Dark;
    let colors = DiffColors::of(dark_mode);
    let viewport = ctx.content_rect();
    let modal_w = (viewport.width() - 48.0).clamp(520.0, 1520.0);
    let modal_h = (viewport.height() - 48.0).clamp(360.0, 980.0);

    let (nav_items, nav_idx) = info
        .status
        .map(|st| build_nav_list(&selected, st))
        .unwrap_or_default();

    let mut outcome = DiffModalOutcome::None;
    let mut close_requested = false;

    // Keyboard shortcuts while the diff comparison modal is open.
    let (key_next_change, key_prev_change, key_prev_file, key_next_file, key_copy) =
        ctx.input(|input| {
            let f7 = input.key_pressed(Key::F7);
            let next_change = f7 && !input.modifiers.shift;
            let prev_change = f7 && input.modifiers.shift;
            let prev_file = input.modifiers.alt && input.key_pressed(Key::ArrowLeft);
            let next_file = input.modifiers.alt && input.key_pressed(Key::ArrowRight);
            let copy = input.modifiers.command && input.key_pressed(Key::C);
            (next_change, prev_change, prev_file, next_file, copy)
        });
    if key_next_change {
        state.jump_change(1);
    } else if key_prev_change {
        state.jump_change(-1);
    }
    if key_prev_file
        && let Some(idx) = nav_idx
        && idx > 0
    {
        outcome = DiffModalOutcome::SelectAndLoad(nav_items[idx - 1].clone());
    } else if key_next_file
        && let Some(idx) = nav_idx
        && idx + 1 < nav_items.len()
    {
        outcome = DiffModalOutcome::SelectAndLoad(nav_items[idx + 1].clone());
    }
    if key_copy
        && let Some(row_idx) = state.selected_row
        && let Some(parsed) = &state.parsed
        && let Some(DiffRow::Pair {
            left_text,
            right_text,
            ..
        }) = parsed.rows.get(row_idx)
    {
        let text = if !right_text.is_empty() {
            right_text.clone()
        } else {
            left_text.clone()
        };
        ctx.copy_text(text);
    }

    let modal_id = Id::new("git diff comparison modal");
    let open_t = ctx.animate_bool_with_time(modal_id.with("backdrop"), true, 0.14);
    let modal = egui::Modal::new(modal_id)
        .frame(popup(palette, Margin::same(0)))
        .backdrop_color(palette.backdrop.gamma_multiply(open_t))
        .show(ctx, |ui| {
            let language = Language::in_ui(ui);
            ui.set_min_size(vec2(modal_w, modal_h));
            ui.set_max_size(vec2(modal_w, modal_h));
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);

            // ── Row 1: Header with file title, badge, scope, stats, close ──
            let header = egui::Frame::new()
                .fill(palette.canvas)
                .corner_radius(egui::CornerRadius {
                    nw: 12,
                    ne: 12,
                    sw: 0,
                    se: 0,
                })
                .inner_margin(Margin::symmetric(14, 9))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), 26.0),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            if Icon::Close
                                .button(ui, language.text("关闭差异"), palette.muted)
                                .clicked()
                            {
                                close_requested = true;
                            }
                            if let Some(diff) = info.diff {
                                if diff.removed > 0 {
                                    ui.label(
                                        RichText::new(format!("-{}", diff.removed))
                                            .size(12.0)
                                            .monospace()
                                            .color(palette.danger)
                                            .strong(),
                                    );
                                }
                                if diff.added > 0 {
                                    ui.label(
                                        RichText::new(format!("+{}", diff.added))
                                            .size(12.0)
                                            .monospace()
                                            .color(palette.added)
                                            .strong(),
                                    );
                                }
                            }
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let (scope_label, scope_color) = match &selected {
                                    SelectedDiff::Staged(_) => {
                                        (language.text("已暂存"), palette.added)
                                    }
                                    SelectedDiff::Unstaged(_) => {
                                        (language.text("更改"), palette.accent)
                                    }
                                    SelectedDiff::Untracked(_) => {
                                        (language.text("未跟踪"), palette.added)
                                    }
                                    SelectedDiff::Commit(_) => {
                                        (language.text("提交"), palette.accent)
                                    }
                                };
                                egui::Frame::new()
                                    .fill(palette.tint)
                                    .corner_radius(5)
                                    .inner_margin(Margin::symmetric(6, 2))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(scope_label)
                                                .size(11.0)
                                                .color(scope_color)
                                                .strong(),
                                        );
                                    });

                                let raw_title = match &selected {
                                    SelectedDiff::Staged(p)
                                    | SelectedDiff::Unstaged(p)
                                    | SelectedDiff::Untracked(p) => p.as_str(),
                                    SelectedDiff::Commit(h) => info
                                        .status
                                        .and_then(|st| {
                                            st.commits
                                                .iter()
                                                .find(|c| &c.hash == h)
                                                .map(|c| c.subject.as_str())
                                        })
                                        .unwrap_or(h.as_str()),
                                };
                                if matches!(selected, SelectedDiff::Commit(_)) {
                                    if let Some(short) = info
                                        .diff
                                        .map(|d| d.title.as_str())
                                        .or_else(|| match &selected {
                                            SelectedDiff::Commit(h) => Some(&h[..h.len().min(8)]),
                                            _ => None,
                                        })
                                    {
                                        ui.label(
                                            RichText::new(short)
                                                .size(12.0)
                                                .monospace()
                                                .color(palette.accent)
                                                .strong(),
                                        );
                                    }
                                    ui.add(
                                        Label::new(
                                            RichText::new(raw_title)
                                                .size(13.5)
                                                .color(palette.ink)
                                                .strong(),
                                        )
                                        .truncate(),
                                    );
                                } else {
                                    let path = Path::new(raw_title);
                                    let file_name = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(raw_title);
                                    let parent =
                                        path.parent().and_then(|p| p.to_str()).unwrap_or_default();
                                    ui.label(
                                        RichText::new(file_name)
                                            .size(14.0)
                                            .color(palette.ink)
                                            .strong(),
                                    );
                                    if !parent.is_empty() {
                                        ui.add(
                                            Label::new(
                                                RichText::new(parent)
                                                    .size(12.0)
                                                    .color(palette.muted),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(raw_title);
                                    }
                                }
                            });
                        },
                    );
                });
            ui.painter().hline(
                header.response.rect.x_range(),
                header.response.rect.bottom() - 0.5,
                Stroke::new(1.0, palette.line),
            );

            // ── Row 2: Rider-style toolbar (view mode, collapse, nav, stage/unstage) ──
            let toolbar = egui::Frame::new()
                .fill(palette.paper)
                .inner_margin(Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), 26.0),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            ui.spacing_mut().button_padding = vec2(8.0, 3.0);

                            // Refresh button
                            if ui
                                .add_enabled_ui(!info.diff_loading, |ui| {
                                    Icon::refresh_button(
                                        ui,
                                        language.text("刷新差异"),
                                        palette.muted,
                                        info.diff_loading,
                                    )
                                })
                                .inner
                                .clicked()
                            {
                                outcome = DiffModalOutcome::SelectAndLoad(selected.clone());
                            }

                            // Copy diff button
                            if let Some(diff) = info.diff
                                && !diff.diff.is_empty()
                                && Icon::Copy
                                    .button(ui, language.text("复制差异"), palette.muted)
                                    .clicked()
                            {
                                ctx.copy_text(diff.diff.clone());
                            }

                            // Stage / Unstage quick action
                            ui.add_enabled_ui(!info.action_pending, |ui| match &selected {
                                SelectedDiff::Staged(path) => {
                                    if ui
                                        .add(
                                            Button::new(
                                                RichText::new(format!(
                                                    "− {}",
                                                    language.text("取消暂存")
                                                ))
                                                .size(12.0)
                                                .color(palette.muted),
                                            )
                                            .small(),
                                        )
                                        .clicked()
                                    {
                                        outcome = DiffModalOutcome::Command(GitCommand::RunAction(
                                            GitAction::Unstage {
                                                paths: vec![path.clone()],
                                            },
                                        ));
                                    }
                                }
                                SelectedDiff::Unstaged(path) | SelectedDiff::Untracked(path) => {
                                    if ui
                                        .add(
                                            Button::new(
                                                RichText::new(format!(
                                                    "+ {}",
                                                    language.text("暂存")
                                                ))
                                                .size(12.0)
                                                .color(palette.added),
                                            )
                                            .small(),
                                        )
                                        .clicked()
                                    {
                                        outcome = DiffModalOutcome::Command(GitCommand::RunAction(
                                            GitAction::Stage {
                                                paths: vec![path.clone()],
                                            },
                                        ));
                                    }
                                }
                                SelectedDiff::Commit(_) => {}
                            });

                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                ui.spacing_mut().button_padding = vec2(8.0, 3.0);

                                // View mode segmented buttons
                                for (mode, label) in [
                                    (DiffViewMode::SideBySide, "并排对比"),
                                    (DiffViewMode::Unified, "统一视图"),
                                ] {
                                    let active = state.mode == mode;
                                    let btn = Button::new(
                                        RichText::new(language.text(label)).size(12.0).color(
                                            if active {
                                                palette.accent
                                            } else {
                                                palette.muted
                                            },
                                        ),
                                    )
                                    .small()
                                    .selected(active)
                                    .frame_when_inactive(active);
                                    if ui.add(btn).clicked() && !active {
                                        state.mode = mode;
                                        state.h_scroll = 0.0;
                                        state.content_max_width = 0.0;
                                    }
                                }

                                // Collapse unchanged toggle (when file has long unchanged spans)
                                let has_collapsible = state
                                    .parsed
                                    .as_ref()
                                    .is_some_and(|p| p.has_collapsible_context);
                                if has_collapsible {
                                    ui.separator();
                                    let active = state.collapse_unchanged;
                                    let btn = Button::new(
                                        RichText::new(language.text("折叠未改动"))
                                            .size(12.0)
                                            .color(if active {
                                                palette.accent
                                            } else {
                                                palette.muted
                                            }),
                                    )
                                    .small()
                                    .selected(active)
                                    .frame_when_inactive(active);
                                    if ui.add(btn).clicked() {
                                        state.collapse_unchanged = !state.collapse_unchanged;
                                        state.expanded_folds.clear();
                                    }
                                }

                                // Change block navigation (↑ / ↓)
                                let change_count =
                                    state.parsed.as_ref().map_or(0, |p| p.change_blocks.len());
                                if change_count > 0 {
                                    ui.separator();
                                    if ui
                                        .add(Button::new(RichText::new("↑").size(12.0)).small())
                                        .on_hover_text(language.text("上一处差异 (Shift+F7)"))
                                        .clicked()
                                    {
                                        state.jump_change(-1);
                                    }
                                    let current_display =
                                        (state.current_change.min(change_count - 1)) + 1;
                                    ui.label(
                                        RichText::new(format!(
                                            "{current_display} / {change_count}"
                                        ))
                                        .size(11.5)
                                        .monospace()
                                        .color(palette.muted),
                                    );
                                    if ui
                                        .add(Button::new(RichText::new("↓").size(12.0)).small())
                                        .on_hover_text(language.text("下一处差异 (F7)"))
                                        .clicked()
                                    {
                                        state.jump_change(1);
                                    }
                                }

                                // File navigation (◀ / ▶)
                                if let Some(idx) = nav_idx
                                    && nav_items.len() > 1
                                {
                                    ui.separator();
                                    if ui
                                        .add_enabled(
                                            idx > 0,
                                            Button::new(RichText::new("◀").size(11.0)).small(),
                                        )
                                        .on_hover_text(language.text("上一个文件 (Alt+←)"))
                                        .clicked()
                                    {
                                        outcome = DiffModalOutcome::SelectAndLoad(
                                            nav_items[idx - 1].clone(),
                                        );
                                    }
                                    ui.label(
                                        RichText::new(format!("{} / {}", idx + 1, nav_items.len()))
                                            .size(11.5)
                                            .monospace()
                                            .color(palette.muted),
                                    );
                                    if ui
                                        .add_enabled(
                                            idx + 1 < nav_items.len(),
                                            Button::new(RichText::new("▶").size(11.0)).small(),
                                        )
                                        .on_hover_text(language.text("下一个文件 (Alt+→)"))
                                        .clicked()
                                    {
                                        outcome = DiffModalOutcome::SelectAndLoad(
                                            nav_items[idx + 1].clone(),
                                        );
                                    }
                                }
                            });
                        },
                    );
                });
            ui.painter().hline(
                toolbar.response.rect.x_range(),
                toolbar.response.rect.bottom() - 0.5,
                Stroke::new(1.0, palette.line),
            );

            // ── Status states: loading / error / binary / empty ──
            if info.diff_loading {
                ui.centered_and_justified(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new(language.text("正在加载差异…"))
                                .size(13.0)
                                .color(palette.muted),
                        );
                    });
                });
                return;
            }
            if let Some(error) = info.diff_error {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(error).size(13.0).color(palette.danger));
                });
                return;
            }
            let Some(diff) = info.diff else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(language.text("没有文本差异。"))
                            .size(13.0)
                            .color(palette.muted),
                    );
                });
                return;
            };
            if diff.binary {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(language.text("二进制文件，无法显示文本差异。"))
                            .size(13.0)
                            .color(palette.muted),
                    );
                });
                return;
            }
            if diff.diff.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new(language.text("没有文本差异。"))
                            .size(13.0)
                            .color(palette.muted),
                    );
                });
                return;
            }

            // ── Side-by-side column header bar ──
            if state.mode == DiffViewMode::SideBySide {
                let (left_title, right_title) = match &selected {
                    SelectedDiff::Staged(_) => (language.text("HEAD"), language.text("已暂存")),
                    SelectedDiff::Unstaged(_) => {
                        (language.text("暂存区 / HEAD"), language.text("工作区"))
                    }
                    SelectedDiff::Untracked(_) => (
                        language.text("原文件（无）"),
                        language.text("工作区（新文件）"),
                    ),
                    SelectedDiff::Commit(_) => (language.text("提交前"), language.text("提交后")),
                };
                let col_hdr = egui::Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(0, 4))
                    .show(ui, |ui| {
                        let full_w = ui.available_width();
                        let code_area_w = (full_w - OVERVIEW_WIDTH).max(200.0);
                        let pane_w = ((code_area_w - DIVIDER_BAND_WIDTH) * 0.5).max(80.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(full_w, 18.0), Sense::hover());
                        let p = ui.painter();
                        p.text(
                            pos2(rect.left() + 14.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            left_title,
                            FontId::proportional(11.5),
                            palette.muted,
                        );
                        let right_x = rect.left() + pane_w + DIVIDER_BAND_WIDTH;
                        p.vline(
                            rect.left() + pane_w + DIVIDER_BAND_WIDTH * 0.5,
                            rect.y_range(),
                            Stroke::new(1.0, palette.line),
                        );
                        p.text(
                            pos2(right_x + 14.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            right_title,
                            FontId::proportional(11.5),
                            palette.muted,
                        );
                    });
                ui.painter().hline(
                    col_hdr.response.rect.x_range(),
                    col_hdr.response.rect.bottom() - 0.5,
                    Stroke::new(1.0, palette.line),
                );
            }

            if diff.truncated {
                egui::Frame::new()
                    .fill(palette.notice)
                    .inner_margin(Margin::symmetric(14, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(language.text("仅显示部分差异"))
                                .size(11.5)
                                .color(palette.muted),
                        );
                    });
            }

            // ── Main diff viewport + Overview Ruler ──
            let body_rect = ui.available_rect_before_wrap();
            let hbar_height = if state.content_max_width > 0.0 {
                12.0
            } else {
                0.0
            };
            let main_h = (body_rect.height() - hbar_height).max(80.0);

            // Capture horizontal scroll gestures (trackpad pan / Shift+wheel) over the viewport.
            let pointer_in_body = ctx
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|pos| body_rect.contains(pos));
            if pointer_in_body {
                let dx = ctx.input(|i| i.smooth_scroll_delta.x);
                if dx.abs() > 0.1 {
                    state.h_scroll = (state.h_scroll - dx).max(0.0);
                }
            }

            let mut pane_code_width = 0.0f32;
            ui.allocate_ui_with_layout(
                vec2(body_rect.width(), main_h),
                Layout::left_to_right(Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let code_area_w = (body_rect.width() - OVERVIEW_WIDTH).max(200.0);

                    // Left/main scrollable diff body
                    ui.allocate_ui_with_layout(
                        vec2(code_area_w, main_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_clip_rect(
                                ui.clip_rect().intersect(ui.available_rect_before_wrap()),
                            );
                            match state.mode {
                                DiffViewMode::SideBySide => {
                                    pane_code_width = render_side_by_side(
                                        ui,
                                        state,
                                        palette,
                                        colors,
                                        dark_mode,
                                        language,
                                        code_area_w,
                                    );
                                }
                                DiffViewMode::Unified => {
                                    pane_code_width = render_unified(
                                        ui,
                                        state,
                                        palette,
                                        colors,
                                        dark_mode,
                                        language,
                                        code_area_w,
                                    );
                                }
                            }
                        },
                    );

                    // Right Rider-style Overview Ruler
                    render_overview_ruler(ui, state, palette, colors, vec2(OVERVIEW_WIDTH, main_h));
                },
            );

            // Clamp horizontal scroll & draw bottom slim scrollbar when needed.
            let max_h = (state.content_max_width - pane_code_width).max(0.0);
            if max_h <= 1.0 {
                state.h_scroll = 0.0;
            } else {
                state.h_scroll = state.h_scroll.clamp(0.0, max_h);
            }
            if hbar_height > 0.0 {
                render_horizontal_scrollbar(
                    ui,
                    &mut state.h_scroll,
                    max_h,
                    pane_code_width,
                    state.content_max_width,
                    body_rect.width() - OVERVIEW_WIDTH,
                    hbar_height,
                    palette,
                );
            }
        });

    if close_requested || modal.should_close() {
        state.close();
    }
    outcome
}

fn gutter_width_for(max_line_num: usize) -> f32 {
    let digits = if max_line_num == 0 {
        2
    } else {
        (max_line_num.ilog10() as usize + 1).max(2)
    };
    ((digits as f32) * 8.0 + 18.0).max(GUTTER_MIN_WIDTH)
}

fn build_side_by_side_items(
    parsed: &ParsedDiff,
    collapse: bool,
    expanded_folds: &HashSet<usize>,
) -> Vec<DisplayItem> {
    if !collapse {
        return (0..parsed.rows.len()).map(DisplayItem::Row).collect();
    }
    let mut items = Vec::with_capacity(parsed.rows.len());
    let mut i = 0;
    let total = parsed.rows.len();
    while i < total {
        if !matches!(
            parsed.rows[i],
            DiffRow::Pair {
                kind: PairKind::Context,
                ..
            }
        ) {
            items.push(DisplayItem::Row(i));
            i += 1;
            continue;
        }
        let run_start = i;
        while i < total
            && matches!(
                parsed.rows[i],
                DiffRow::Pair {
                    kind: PairKind::Context,
                    ..
                }
            )
        {
            i += 1;
        }
        let run_end = i;
        let at_start = run_start == 0
            || matches!(
                parsed.rows[run_start - 1],
                DiffRow::FileHeader { .. }
                    | DiffRow::HunkHeader { .. }
                    | DiffRow::CommitMeta { .. }
            );
        let at_end = run_end == total
            || matches!(
                parsed.rows[run_end],
                DiffRow::FileHeader { .. } | DiffRow::HunkHeader { .. }
            );
        let keep_head = if at_start { 0 } else { FOLD_KEEP_CONTEXT };
        let keep_tail = if at_end { 0 } else { FOLD_KEEP_CONTEXT };
        let fold_start = run_start + keep_head;
        let fold_end = run_end.saturating_sub(keep_tail);
        if fold_end > fold_start
            && fold_end - fold_start >= FOLD_MIN_SKIP
            && !expanded_folds.contains(&fold_start)
        {
            for idx in run_start..fold_start {
                items.push(DisplayItem::Row(idx));
            }
            items.push(DisplayItem::Folded {
                fold_id: fold_start,
                skipped_lines: fold_end - fold_start,
            });
            for idx in fold_end..run_end {
                items.push(DisplayItem::Row(idx));
            }
        } else {
            for idx in run_start..run_end {
                items.push(DisplayItem::Row(idx));
            }
        }
    }
    items
}

fn build_unified_items(
    parsed: &ParsedDiff,
    collapse: bool,
    expanded_folds: &HashSet<usize>,
) -> Vec<UnifiedDisplayItem> {
    let base = build_side_by_side_items(parsed, collapse, expanded_folds);
    let mut out = Vec::with_capacity(base.len());
    let mut i = 0;
    while i < base.len() {
        match &base[i] {
            DisplayItem::Folded {
                fold_id,
                skipped_lines,
            } => {
                out.push(UnifiedDisplayItem::Folded {
                    fold_id: *fold_id,
                    skipped_lines: *skipped_lines,
                });
                i += 1;
            }
            DisplayItem::Row(row_idx) => match &parsed.rows[*row_idx] {
                DiffRow::FileHeader { path } => {
                    out.push(UnifiedDisplayItem::FileHeader { path: path.clone() });
                    i += 1;
                }
                DiffRow::HunkHeader { header } => {
                    out.push(UnifiedDisplayItem::HunkHeader {
                        header: header.clone(),
                    });
                    i += 1;
                }
                DiffRow::CommitMeta { text } => {
                    out.push(UnifiedDisplayItem::CommitMeta { text: text.clone() });
                    i += 1;
                }
                DiffRow::Pair {
                    lang,
                    left_line,
                    right_line,
                    right_text,
                    kind: PairKind::Context,
                    ..
                } => {
                    out.push(UnifiedDisplayItem::Line {
                        source_row: *row_idx,
                        lang: lang.clone(),
                        old_line: *left_line,
                        new_line: *right_line,
                        prefix: ' ',
                        text: right_text.clone(),
                        emph: Vec::new(),
                        kind: PairKind::Context,
                    });
                    i += 1;
                }
                DiffRow::Pair { .. } => {
                    // Group contiguous changed pairs so deletions appear before additions
                    // just like standard unified diff, while keeping inline emphasis!
                    let mut group_indices = Vec::new();
                    while i < base.len() {
                        if let DisplayItem::Row(r_idx) = base[i]
                            && let DiffRow::Pair { kind, .. } = parsed.rows[r_idx]
                            && kind != PairKind::Context
                        {
                            group_indices.push(r_idx);
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    for &r_idx in &group_indices {
                        if let DiffRow::Pair {
                            lang,
                            left_line: Some(ln),
                            left_text,
                            left_emph,
                            ..
                        } = &parsed.rows[r_idx]
                        {
                            out.push(UnifiedDisplayItem::Line {
                                source_row: r_idx,
                                lang: lang.clone(),
                                old_line: Some(*ln),
                                new_line: None,
                                prefix: '-',
                                text: left_text.clone(),
                                emph: left_emph.clone(),
                                kind: PairKind::Removed,
                            });
                        }
                    }
                    for &r_idx in &group_indices {
                        if let DiffRow::Pair {
                            lang,
                            right_line: Some(rn),
                            right_text,
                            right_emph,
                            ..
                        } = &parsed.rows[r_idx]
                        {
                            out.push(UnifiedDisplayItem::Line {
                                source_row: r_idx,
                                lang: lang.clone(),
                                old_line: None,
                                new_line: Some(*rn),
                                prefix: '+',
                                text: right_text.clone(),
                                emph: right_emph.clone(),
                                kind: PairKind::Added,
                            });
                        }
                    }
                }
            },
        }
    }
    out
}

fn render_side_by_side(
    ui: &mut Ui,
    state: &mut DiffModalState,
    palette: Palette,
    colors: DiffColors,
    dark_mode: bool,
    language: Language,
    total_w: f32,
) -> f32 {
    let Some(parsed) = state.parsed.as_ref() else {
        return total_w;
    };
    let items = build_side_by_side_items(parsed, state.collapse_unchanged, &state.expanded_folds);
    let gutter_w = gutter_width_for(parsed.max_line_num);
    let pane_w = ((total_w - DIVIDER_BAND_WIDTH) * 0.5).max(120.0);
    let code_w = (pane_w - gutter_w).max(40.0);

    // Compute prefix sums of row heights so mixed heights stay exact & O(log n) clippable.
    let mut y_offsets = Vec::with_capacity(items.len() + 1);
    let mut acc_y = 0.0f32;
    for item in &items {
        y_offsets.push(acc_y);
        acc_y += match item {
            DisplayItem::Folded { .. } => HEADER_ROW_HEIGHT,
            DisplayItem::Row(r_idx) => match parsed.rows[*r_idx] {
                DiffRow::FileHeader { .. } | DiffRow::HunkHeader { .. } => HEADER_ROW_HEIGHT,
                DiffRow::CommitMeta { .. } | DiffRow::Pair { .. } => ROW_HEIGHT,
            },
        };
    }
    y_offsets.push(acc_y);
    let total_h = acc_y.max(ROW_HEIGHT);

    let focused_block = parsed.change_blocks.get(state.current_change).copied();
    let scroll_target = state.scroll_to_source_row.take();
    let mut expand_fold: Option<usize> = None;
    let mut clicked_row: Option<usize> = None;
    let mut copy_text: Option<String> = None;
    let mut observed_max_w = state.content_max_width;
    let h_scroll = state.h_scroll;

    egui::ScrollArea::vertical()
        .id_salt((
            "git diff sbs scroll",
            state.cached_key.as_ref().map(|k| k.0),
        ))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (canvas_rect, _) = ui.allocate_exact_size(vec2(total_w, total_h), Sense::hover());
            let clip = ui.clip_rect().intersect(canvas_rect);

            // Handle programmatic scroll to a target source row.
            if let Some(target_row) = scroll_target {
                let display_idx = items.iter().position(|item| match item {
                    DisplayItem::Row(idx) => *idx >= target_row,
                    DisplayItem::Folded { fold_id, .. } => *fold_id >= target_row,
                });
                if let Some(d_idx) = display_idx {
                    let y0 = canvas_rect.top() + y_offsets[d_idx];
                    let y1 = canvas_rect.top() + y_offsets[d_idx + 1];
                    ui.scroll_to_rect(
                        Rect::from_x_y_ranges(canvas_rect.x_range(), y0..=y1),
                        Some(Align::Center),
                    );
                }
            }

            let rel_top = (clip.top() - canvas_rect.top()).max(0.0);
            let rel_bottom = (clip.bottom() - canvas_rect.top()).max(0.0);
            let first = y_offsets
                .partition_point(|&y| y <= rel_top)
                .saturating_sub(1);

            let left_gutter_x = canvas_rect.left();
            let left_code_x = left_gutter_x + gutter_w;
            let divider_x = left_gutter_x + pane_w;
            let right_gutter_x = divider_x + DIVIDER_BAND_WIDTH;
            let right_code_x = right_gutter_x + gutter_w;
            let left_code_clip =
                Rect::from_x_y_ranges(left_code_x..=divider_x, clip.y_range()).intersect(clip);
            let right_code_clip =
                Rect::from_x_y_ranges(right_code_x..=canvas_rect.right(), clip.y_range())
                    .intersect(clip);

            let painter = ui.painter();

            // Base gutter backgrounds & center divider track across the visible clip.
            painter.rect_filled(
                Rect::from_x_y_ranges(left_gutter_x..=left_code_x, clip.y_range()),
                0.0,
                palette.canvas,
            );
            painter.rect_filled(
                Rect::from_x_y_ranges(right_gutter_x..=right_code_x, clip.y_range()),
                0.0,
                palette.canvas,
            );
            painter.rect_filled(
                Rect::from_x_y_ranges(divider_x..=right_gutter_x, clip.y_range()),
                0.0,
                palette.canvas,
            );

            for (d_idx, item) in items.iter().enumerate().skip(first) {
                let y0 = y_offsets[d_idx];
                if y0 > rel_bottom {
                    break;
                }
                let y1 = y_offsets[d_idx + 1];
                let row_rect = Rect::from_x_y_ranges(
                    canvas_rect.x_range(),
                    (canvas_rect.top() + y0)..=(canvas_rect.top() + y1),
                );

                match item {
                    DisplayItem::Folded {
                        fold_id,
                        skipped_lines,
                    } => {
                        let resp = ui.interact(
                            row_rect,
                            ui.id().with(("diff fold", *fold_id)),
                            Sense::click(),
                        );
                        let bg = if resp.hovered() {
                            palette.tint
                        } else {
                            colors.fold_bg
                        };
                        painter.rect_filled(row_rect, 0.0, bg);
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.top(),
                            Stroke::new(1.0, palette.line),
                        );
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.bottom(),
                            Stroke::new(1.0, palette.line),
                        );
                        let label = language.format(
                            "⋯ 展开 {count} 行未改动代码 ⋯",
                            &[("count", &skipped_lines.to_string())],
                        );
                        painter.text(
                            row_rect.center(),
                            Align2::CENTER_CENTER,
                            label,
                            FontId::proportional(11.5),
                            palette.accent,
                        );
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            expand_fold = Some(*fold_id);
                        }
                    }
                    DisplayItem::Row(r_idx) => match &parsed.rows[*r_idx] {
                        DiffRow::FileHeader { path } => {
                            painter.rect_filled(row_rect, 0.0, colors.file_header_bg);
                            painter.hline(
                                row_rect.x_range(),
                                row_rect.top(),
                                Stroke::new(1.0, palette.line),
                            );
                            painter.hline(
                                row_rect.x_range(),
                                row_rect.bottom(),
                                Stroke::new(1.0, palette.line),
                            );
                            painter.text(
                                pos2(row_rect.left() + 12.0, row_rect.center().y),
                                Align2::LEFT_CENTER,
                                path,
                                FontId::monospace(12.0),
                                palette.ink,
                            );
                        }
                        DiffRow::HunkHeader { header } => {
                            painter.rect_filled(row_rect, 0.0, colors.hunk_bg);
                            painter.hline(
                                row_rect.x_range(),
                                row_rect.top(),
                                Stroke::new(1.0, palette.line),
                            );
                            painter.hline(
                                row_rect.x_range(),
                                row_rect.bottom(),
                                Stroke::new(1.0, palette.line),
                            );
                            painter.text(
                                pos2(row_rect.left() + 12.0, row_rect.center().y),
                                Align2::LEFT_CENTER,
                                header,
                                FontId::monospace(11.5),
                                palette.accent,
                            );
                        }
                        DiffRow::CommitMeta { text } => {
                            painter.rect_filled(row_rect, 0.0, palette.canvas);
                            painter.text(
                                pos2(row_rect.left() + 12.0, row_rect.center().y),
                                Align2::LEFT_CENTER,
                                text,
                                FontId::monospace(11.5),
                                palette.muted,
                            );
                        }
                        DiffRow::Pair {
                            lang,
                            left_line,
                            right_line,
                            left_text,
                            right_text,
                            left_emph,
                            right_emph,
                            kind,
                        } => {
                            let y_range = row_rect.y_range();
                            let left_pane =
                                Rect::from_x_y_ranges(left_gutter_x..=divider_x, y_range);
                            let left_gutter =
                                Rect::from_x_y_ranges(left_gutter_x..=left_code_x, y_range);
                            let right_pane = Rect::from_x_y_ranges(
                                right_gutter_x..=canvas_rect.right(),
                                y_range,
                            );
                            let right_gutter =
                                Rect::from_x_y_ranges(right_gutter_x..=right_code_x, y_range);
                            let center_band =
                                Rect::from_x_y_ranges(divider_x..=right_gutter_x, y_range);

                            // Paint row fills
                            let (l_bg, l_gutter, r_bg, r_gutter, band_color) = match kind {
                                PairKind::Context => (
                                    Color32::TRANSPARENT,
                                    Color32::TRANSPARENT,
                                    Color32::TRANSPARENT,
                                    Color32::TRANSPARENT,
                                    Color32::TRANSPARENT,
                                ),
                                PairKind::Added => (
                                    colors.empty_slot,
                                    colors.empty_slot,
                                    colors.added_bg,
                                    colors.added_gutter,
                                    colors.added_bar,
                                ),
                                PairKind::Removed => (
                                    colors.removed_bg,
                                    colors.removed_gutter,
                                    colors.empty_slot,
                                    colors.empty_slot,
                                    colors.removed_bar,
                                ),
                                PairKind::Modified => (
                                    colors.removed_bg,
                                    colors.removed_gutter,
                                    colors.added_bg,
                                    colors.added_gutter,
                                    colors.modified_bar,
                                ),
                            };
                            if l_bg != Color32::TRANSPARENT {
                                painter.rect_filled(left_pane, 0.0, l_bg);
                            }
                            if l_gutter != Color32::TRANSPARENT {
                                painter.rect_filled(left_gutter, 0.0, l_gutter);
                            }
                            if r_bg != Color32::TRANSPARENT {
                                painter.rect_filled(right_pane, 0.0, r_bg);
                            }
                            if r_gutter != Color32::TRANSPARENT {
                                painter.rect_filled(right_gutter, 0.0, r_gutter);
                            }
                            if band_color != Color32::TRANSPARENT {
                                painter.rect_filled(
                                    center_band.shrink2(vec2(2.0, 0.0)),
                                    0.0,
                                    band_color.gamma_multiply(0.55),
                                );
                            }

                            // Focused change block accent marker
                            let in_focus = focused_block
                                .is_some_and(|b| *r_idx >= b.start_row && *r_idx < b.end_row);
                            if in_focus {
                                painter.rect_filled(
                                    Rect::from_x_y_ranges(
                                        left_gutter_x..=(left_gutter_x + 2.5),
                                        y_range,
                                    ),
                                    0.0,
                                    palette.accent,
                                );
                            }

                            // Selected row outline
                            if state.selected_row == Some(*r_idx) {
                                painter.rect_stroke(
                                    row_rect,
                                    0.0,
                                    Stroke::new(1.0, colors.selected_outline.gamma_multiply(0.55)),
                                    egui::StrokeKind::Inside,
                                );
                            }

                            // Line numbers
                            if let Some(ln) = left_line {
                                painter.text(
                                    pos2(left_code_x - 8.0, row_rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    ln.to_string(),
                                    FontId::monospace(11.5),
                                    palette.muted,
                                );
                            }
                            if let Some(rn) = right_line {
                                painter.text(
                                    pos2(right_code_x - 8.0, row_rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    rn.to_string(),
                                    FontId::monospace(11.5),
                                    palette.muted,
                                );
                            }

                            // Code text with syntax highlight + inline diff emphasis
                            if left_line.is_some() {
                                let job = desktop_syntax::highlight_line_job(
                                    left_text,
                                    lang,
                                    CODE_FONT_SIZE,
                                    ROW_HEIGHT,
                                    dark_mode,
                                    Color32::TRANSPARENT,
                                    colors.removed_emph,
                                    left_emph,
                                );
                                let galley = ui.fonts_mut(|f| f.layout_job(job));
                                observed_max_w = observed_max_w.max(galley.size().x + 20.0);
                                ui.painter().with_clip_rect(left_code_clip).galley(
                                    pos2(left_code_x + 10.0 - h_scroll, row_rect.top()),
                                    galley,
                                    palette.ink,
                                );
                            }
                            if right_line.is_some() {
                                let job = desktop_syntax::highlight_line_job(
                                    right_text,
                                    lang,
                                    CODE_FONT_SIZE,
                                    ROW_HEIGHT,
                                    dark_mode,
                                    Color32::TRANSPARENT,
                                    colors.added_emph,
                                    right_emph,
                                );
                                let galley = ui.fonts_mut(|f| f.layout_job(job));
                                observed_max_w = observed_max_w.max(galley.size().x + 20.0);
                                ui.painter().with_clip_rect(right_code_clip).galley(
                                    pos2(right_code_x + 10.0 - h_scroll, row_rect.top()),
                                    galley,
                                    palette.ink,
                                );
                            }

                            // Row click / context menu for copying lines
                            let resp = ui.interact(
                                row_rect,
                                ui.id().with(("diff sbs row", *r_idx)),
                                Sense::click(),
                            );
                            if resp.clicked() {
                                clicked_row = Some(*r_idx);
                            }
                            resp.context_menu(|ui| {
                                if left_line.is_some()
                                    && ui.button(language.text("复制旧版本行")).clicked()
                                {
                                    copy_text = Some(left_text.clone());
                                    ui.close();
                                }
                                if right_line.is_some()
                                    && ui.button(language.text("复制新版本行")).clicked()
                                {
                                    copy_text = Some(right_text.clone());
                                    ui.close();
                                }
                            });
                        }
                    },
                }
            }

            // Crisp vertical gutter and center lines on top.
            painter.vline(
                left_code_x - 0.5,
                clip.y_range(),
                Stroke::new(1.0, palette.line),
            );
            painter.vline(divider_x, clip.y_range(), Stroke::new(1.0, palette.line));
            painter.vline(
                right_gutter_x,
                clip.y_range(),
                Stroke::new(1.0, palette.line),
            );
            painter.vline(
                right_code_x - 0.5,
                clip.y_range(),
                Stroke::new(1.0, palette.line),
            );
        });

    state.content_max_width = observed_max_w;
    if let Some(fold_id) = expand_fold {
        state.expanded_folds.insert(fold_id);
    }
    if let Some(r) = clicked_row {
        state.selected_row = Some(r);
    }
    if let Some(text) = copy_text {
        ui.ctx().copy_text(text);
    }
    code_w
}

fn render_unified(
    ui: &mut Ui,
    state: &mut DiffModalState,
    palette: Palette,
    colors: DiffColors,
    dark_mode: bool,
    language: Language,
    total_w: f32,
) -> f32 {
    let Some(parsed) = state.parsed.as_ref() else {
        return total_w;
    };
    let items = build_unified_items(parsed, state.collapse_unchanged, &state.expanded_folds);
    let single_gutter = gutter_width_for(parsed.max_line_num);
    let dual_gutter = single_gutter * 2.0;
    let sign_w = 18.0;
    let code_w = (total_w - dual_gutter - sign_w).max(80.0);

    let mut y_offsets = Vec::with_capacity(items.len() + 1);
    let mut acc_y = 0.0f32;
    for item in &items {
        y_offsets.push(acc_y);
        acc_y += match item {
            UnifiedDisplayItem::FileHeader { .. }
            | UnifiedDisplayItem::HunkHeader { .. }
            | UnifiedDisplayItem::Folded { .. } => HEADER_ROW_HEIGHT,
            UnifiedDisplayItem::CommitMeta { .. } | UnifiedDisplayItem::Line { .. } => ROW_HEIGHT,
        };
    }
    y_offsets.push(acc_y);
    let total_h = acc_y.max(ROW_HEIGHT);

    let focused_block = parsed.change_blocks.get(state.current_change).copied();
    let scroll_target = state.scroll_to_source_row.take();
    let mut expand_fold: Option<usize> = None;
    let mut clicked_row: Option<usize> = None;
    let mut copy_text: Option<String> = None;
    let mut observed_max_w = state.content_max_width;
    let h_scroll = state.h_scroll;

    egui::ScrollArea::vertical()
        .id_salt((
            "git diff uni scroll",
            state.cached_key.as_ref().map(|k| k.0),
        ))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (canvas_rect, _) = ui.allocate_exact_size(vec2(total_w, total_h), Sense::hover());
            let clip = ui.clip_rect().intersect(canvas_rect);

            if let Some(target_row) = scroll_target {
                let display_idx = items.iter().position(|item| match item {
                    UnifiedDisplayItem::Line { source_row, .. } => *source_row >= target_row,
                    UnifiedDisplayItem::Folded { fold_id, .. } => *fold_id >= target_row,
                    _ => false,
                });
                if let Some(d_idx) = display_idx {
                    let y0 = canvas_rect.top() + y_offsets[d_idx];
                    let y1 = canvas_rect.top() + y_offsets[d_idx + 1];
                    ui.scroll_to_rect(
                        Rect::from_x_y_ranges(canvas_rect.x_range(), y0..=y1),
                        Some(Align::Center),
                    );
                }
            }

            let rel_top = (clip.top() - canvas_rect.top()).max(0.0);
            let rel_bottom = (clip.bottom() - canvas_rect.top()).max(0.0);
            let first = y_offsets
                .partition_point(|&y| y <= rel_top)
                .saturating_sub(1);

            let old_x = canvas_rect.left();
            let new_x = old_x + single_gutter;
            let sign_x = old_x + dual_gutter;
            let code_x = sign_x + sign_w;
            let code_clip =
                Rect::from_x_y_ranges(code_x..=canvas_rect.right(), clip.y_range()).intersect(clip);
            let painter = ui.painter();

            painter.rect_filled(
                Rect::from_x_y_ranges(old_x..=sign_x, clip.y_range()),
                0.0,
                palette.canvas,
            );

            for (d_idx, item) in items.iter().enumerate().skip(first) {
                let y0 = y_offsets[d_idx];
                if y0 > rel_bottom {
                    break;
                }
                let y1 = y_offsets[d_idx + 1];
                let row_rect = Rect::from_x_y_ranges(
                    canvas_rect.x_range(),
                    (canvas_rect.top() + y0)..=(canvas_rect.top() + y1),
                );
                match item {
                    UnifiedDisplayItem::Folded {
                        fold_id,
                        skipped_lines,
                    } => {
                        let resp = ui.interact(
                            row_rect,
                            ui.id().with(("diff uni fold", *fold_id)),
                            Sense::click(),
                        );
                        let bg = if resp.hovered() {
                            palette.tint
                        } else {
                            colors.fold_bg
                        };
                        painter.rect_filled(row_rect, 0.0, bg);
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.top(),
                            Stroke::new(1.0, palette.line),
                        );
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.bottom(),
                            Stroke::new(1.0, palette.line),
                        );
                        let label = language.format(
                            "⋯ 展开 {count} 行未改动代码 ⋯",
                            &[("count", &skipped_lines.to_string())],
                        );
                        painter.text(
                            row_rect.center(),
                            Align2::CENTER_CENTER,
                            label,
                            FontId::proportional(11.5),
                            palette.accent,
                        );
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            expand_fold = Some(*fold_id);
                        }
                    }
                    UnifiedDisplayItem::FileHeader { path } => {
                        painter.rect_filled(row_rect, 0.0, colors.file_header_bg);
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.top(),
                            Stroke::new(1.0, palette.line),
                        );
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.bottom(),
                            Stroke::new(1.0, palette.line),
                        );
                        painter.text(
                            pos2(row_rect.left() + 12.0, row_rect.center().y),
                            Align2::LEFT_CENTER,
                            path,
                            FontId::monospace(12.0),
                            palette.ink,
                        );
                    }
                    UnifiedDisplayItem::HunkHeader { header } => {
                        painter.rect_filled(row_rect, 0.0, colors.hunk_bg);
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.top(),
                            Stroke::new(1.0, palette.line),
                        );
                        painter.hline(
                            row_rect.x_range(),
                            row_rect.bottom(),
                            Stroke::new(1.0, palette.line),
                        );
                        painter.text(
                            pos2(row_rect.left() + 12.0, row_rect.center().y),
                            Align2::LEFT_CENTER,
                            header,
                            FontId::monospace(11.5),
                            palette.accent,
                        );
                    }
                    UnifiedDisplayItem::CommitMeta { text } => {
                        painter.rect_filled(row_rect, 0.0, palette.canvas);
                        painter.text(
                            pos2(row_rect.left() + 12.0, row_rect.center().y),
                            Align2::LEFT_CENTER,
                            text,
                            FontId::monospace(11.5),
                            palette.muted,
                        );
                    }
                    UnifiedDisplayItem::Line {
                        source_row,
                        lang,
                        old_line,
                        new_line,
                        prefix,
                        text,
                        emph,
                        kind,
                    } => {
                        let (bg, gutter_bg, emph_bg, prefix_color) = match kind {
                            PairKind::Added => (
                                colors.added_bg,
                                colors.added_gutter,
                                colors.added_emph,
                                palette.added,
                            ),
                            PairKind::Removed => (
                                colors.removed_bg,
                                colors.removed_gutter,
                                colors.removed_emph,
                                palette.danger,
                            ),
                            _ => (
                                Color32::TRANSPARENT,
                                Color32::TRANSPARENT,
                                Color32::TRANSPARENT,
                                palette.muted,
                            ),
                        };
                        if bg != Color32::TRANSPARENT {
                            painter.rect_filled(row_rect, 0.0, bg);
                            painter.rect_filled(
                                Rect::from_x_y_ranges(old_x..=sign_x, row_rect.y_range()),
                                0.0,
                                gutter_bg,
                            );
                        }
                        let in_focus = focused_block
                            .is_some_and(|b| *source_row >= b.start_row && *source_row < b.end_row);
                        if in_focus {
                            painter.rect_filled(
                                Rect::from_x_y_ranges(old_x..=(old_x + 2.5), row_rect.y_range()),
                                0.0,
                                palette.accent,
                            );
                        }
                        if state.selected_row == Some(*source_row) {
                            painter.rect_stroke(
                                row_rect,
                                0.0,
                                Stroke::new(1.0, colors.selected_outline.gamma_multiply(0.55)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if let Some(ln) = old_line {
                            painter.text(
                                pos2(new_x - 8.0, row_rect.center().y),
                                Align2::RIGHT_CENTER,
                                ln.to_string(),
                                FontId::monospace(11.5),
                                palette.muted,
                            );
                        }
                        if let Some(rn) = new_line {
                            painter.text(
                                pos2(sign_x - 8.0, row_rect.center().y),
                                Align2::RIGHT_CENTER,
                                rn.to_string(),
                                FontId::monospace(11.5),
                                palette.muted,
                            );
                        }
                        if *prefix != ' ' {
                            painter.text(
                                pos2(sign_x + sign_w * 0.5, row_rect.center().y),
                                Align2::CENTER_CENTER,
                                prefix.to_string(),
                                FontId::monospace(12.0),
                                prefix_color,
                            );
                        }
                        let job = desktop_syntax::highlight_line_job(
                            text,
                            lang,
                            CODE_FONT_SIZE,
                            ROW_HEIGHT,
                            dark_mode,
                            Color32::TRANSPARENT,
                            emph_bg,
                            emph,
                        );
                        let galley = ui.fonts_mut(|f| f.layout_job(job));
                        observed_max_w = observed_max_w.max(galley.size().x + 20.0);
                        ui.painter().with_clip_rect(code_clip).galley(
                            pos2(code_x + 4.0 - h_scroll, row_rect.top()),
                            galley,
                            palette.ink,
                        );

                        let resp = ui.interact(
                            row_rect,
                            ui.id().with(("diff uni row", d_idx)),
                            Sense::click(),
                        );
                        if resp.clicked() {
                            clicked_row = Some(*source_row);
                        }
                        resp.context_menu(|ui| {
                            if ui.button(language.text("复制当前行")).clicked() {
                                copy_text = Some(text.clone());
                                ui.close();
                            }
                        });
                    }
                }
            }

            painter.vline(
                new_x - 0.5,
                clip.y_range(),
                Stroke::new(1.0, palette.line.gamma_multiply(0.6)),
            );
            painter.vline(sign_x - 0.5, clip.y_range(), Stroke::new(1.0, palette.line));
        });

    state.content_max_width = observed_max_w;
    if let Some(fold_id) = expand_fold {
        state.expanded_folds.insert(fold_id);
    }
    if let Some(r) = clicked_row {
        state.selected_row = Some(r);
    }
    if let Some(text) = copy_text {
        ui.ctx().copy_text(text);
    }
    code_w
}

/// Rider-style overview stripe on the right edge showing where changes sit across the file.
fn render_overview_ruler(
    ui: &mut Ui,
    state: &mut DiffModalState,
    palette: Palette,
    colors: DiffColors,
    size: egui::Vec2,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, palette.canvas);
    painter.vline(
        rect.left() + 0.5,
        rect.y_range(),
        Stroke::new(1.0, palette.line),
    );

    let Some(parsed) = &state.parsed else {
        return;
    };
    let total_rows = parsed.rows.len().max(1) as f32;
    for (idx, block) in parsed.change_blocks.iter().enumerate() {
        let y0 = rect.top() + (block.start_row as f32 / total_rows) * rect.height();
        let y1 = rect.top() + (block.end_row as f32 / total_rows) * rect.height();
        let marker = Rect::from_x_y_ranges(
            (rect.left() + 3.0)..=(rect.right() - 2.0),
            y0..=y1.max(y0 + 3.5),
        );
        let fill = match block.kind {
            PairKind::Added => colors.added_bar,
            PairKind::Removed => colors.removed_bar,
            PairKind::Modified => colors.modified_bar,
            PairKind::Context => palette.muted,
        };
        painter.rect_filled(marker, 1.5, fill);
        if idx == state.current_change {
            painter.rect_stroke(
                marker.expand(1.0),
                1.5,
                Stroke::new(1.0, palette.ink),
                egui::StrokeKind::Outside,
            );
        }
    }

    if (resp.clicked() || resp.dragged())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let ratio = ((pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let target = (ratio * total_rows) as usize;
        state.scroll_to_source_row = Some(target.min(parsed.rows.len().saturating_sub(1)));
        if let Some((b_idx, _)) = parsed
            .change_blocks
            .iter()
            .enumerate()
            .min_by_key(|(_, b)| b.start_row.abs_diff(target))
        {
            state.current_change = b_idx;
        }
    }
    if resp.hovered() && !parsed.change_blocks.is_empty() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_horizontal_scrollbar(
    ui: &mut Ui,
    h_scroll: &mut f32,
    max_scroll: f32,
    viewport_w: f32,
    content_w: f32,
    bar_w: f32,
    bar_h: f32,
    palette: Palette,
) {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(bar_w.max(40.0), bar_h), Sense::click_and_drag());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, palette.canvas);
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, palette.line),
    );
    if max_scroll <= 1.0 || content_w <= 1.0 {
        return;
    }
    let track = rect.shrink2(vec2(6.0, 2.5));
    let thumb_ratio = (viewport_w / content_w).clamp(0.08, 1.0);
    let thumb_w = (track.width() * thumb_ratio).max(28.0);
    let travel = (track.width() - thumb_w).max(1.0);
    let frac = (*h_scroll / max_scroll).clamp(0.0, 1.0);
    let thumb = Rect::from_min_size(
        pos2(track.left() + frac * travel, track.top()),
        vec2(thumb_w, track.height()),
    );
    let thumb_color = if resp.dragged() || resp.hovered() {
        palette.muted
    } else {
        palette.line
    };
    painter.rect_filled(thumb, thumb.height() * 0.5, thumb_color);

    if (resp.clicked() || resp.dragged())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let center_x = (pos.x - track.left() - thumb_w * 0.5).clamp(0.0, travel);
        *h_scroll = (center_x / travel) * max_scroll;
    }
}

/// Parse a unified diff into paired side-by-side rows with inline word diffs.
fn parse_unified_diff(diff: &str, default_path: &str) -> ParsedDiff {
    let mut rows: Vec<DiffRow> = Vec::new();
    let mut current_lang = core_syntax::detect_syntax_from_path(Path::new(default_path));
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;
    let mut max_line_num: usize = 0;
    let mut in_hunk = false;
    let mut file_count = 0usize;
    let mut hunk_count = 0usize;

    let mut pending_removed: Vec<(usize, String)> = Vec::new();
    let mut pending_added: Vec<(usize, String)> = Vec::new();

    let flush_pending = |rows: &mut Vec<DiffRow>,
                         removed: &mut Vec<(usize, String)>,
                         added: &mut Vec<(usize, String)>,
                         lang: &str| {
        if removed.is_empty() && added.is_empty() {
            return;
        }
        let paired = removed.len().min(added.len());
        for idx in 0..paired {
            let (ln, l_text) = &removed[idx];
            let (rn, r_text) = &added[idx];
            let (l_emph, r_emph) = compute_inline_diff(l_text, r_text);
            rows.push(DiffRow::Pair {
                lang: lang.to_owned(),
                left_line: Some(*ln),
                right_line: Some(*rn),
                left_text: l_text.clone(),
                right_text: r_text.clone(),
                left_emph: l_emph,
                right_emph: r_emph,
                kind: PairKind::Modified,
            });
        }
        for (ln, l_text) in removed.drain(..).skip(paired) {
            rows.push(DiffRow::Pair {
                lang: lang.to_owned(),
                left_line: Some(ln),
                right_line: None,
                left_text: l_text,
                right_text: String::new(),
                left_emph: Vec::new(),
                right_emph: Vec::new(),
                kind: PairKind::Removed,
            });
        }
        for (rn, r_text) in added.drain(..).skip(paired) {
            rows.push(DiffRow::Pair {
                lang: lang.to_owned(),
                left_line: None,
                right_line: Some(rn),
                left_text: String::new(),
                right_text: r_text,
                left_emph: Vec::new(),
                right_emph: Vec::new(),
                kind: PairKind::Added,
            });
        }
    };

    for raw_line in diff.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_pending(
                &mut rows,
                &mut pending_removed,
                &mut pending_added,
                &current_lang,
            );
            in_hunk = false;
            file_count += 1;
            let file_path = extract_git_diff_path(rest);
            if !file_path.is_empty() {
                let detected = core_syntax::detect_syntax_from_path(Path::new(&file_path));
                if !detected.is_empty() {
                    current_lang = detected;
                }
                rows.push(DiffRow::FileHeader { path: file_path });
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            flush_pending(
                &mut rows,
                &mut pending_removed,
                &mut pending_added,
                &current_lang,
            );
            let candidate = rest.trim().strip_prefix("b/").unwrap_or(rest.trim());
            if candidate != "/dev/null" {
                let detected = core_syntax::detect_syntax_from_path(Path::new(candidate));
                if !detected.is_empty() {
                    current_lang = detected;
                }
            }
            continue;
        }
        if line.starts_with("--- ")
            || line.starts_with("index ")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("new file mode ")
            || line.starts_with("deleted file mode ")
            || line.starts_with("similarity index ")
            || line.starts_with("rename from ")
            || line.starts_with("rename to ")
        {
            flush_pending(
                &mut rows,
                &mut pending_removed,
                &mut pending_added,
                &current_lang,
            );
            continue;
        }
        if line.starts_with("@@") {
            flush_pending(
                &mut rows,
                &mut pending_removed,
                &mut pending_added,
                &current_lang,
            );
            if let Some((o_start, n_start)) = parse_hunk_header(line) {
                old_line = o_start;
                new_line = n_start;
                in_hunk = true;
                hunk_count += 1;
                rows.push(DiffRow::HunkHeader {
                    header: line.to_owned(),
                });
                continue;
            }
        }
        if !in_hunk {
            // Preserve commit header metadata (Author, Date, message, stat) before first diff.
            if !line.is_empty() || !rows.is_empty() {
                rows.push(DiffRow::CommitMeta {
                    text: line.to_owned(),
                });
            }
            continue;
        }
        if line.starts_with("\\ No newline at end of file") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('-') {
            let ln = old_line;
            old_line += 1;
            max_line_num = max_line_num.max(ln);
            pending_removed.push((ln, rest.to_owned()));
        } else if let Some(rest) = line.strip_prefix('+') {
            let rn = new_line;
            new_line += 1;
            max_line_num = max_line_num.max(rn);
            pending_added.push((rn, rest.to_owned()));
        } else {
            flush_pending(
                &mut rows,
                &mut pending_removed,
                &mut pending_added,
                &current_lang,
            );
            let content = line.strip_prefix(' ').unwrap_or(line);
            let ln = old_line;
            let rn = new_line;
            old_line += 1;
            new_line += 1;
            max_line_num = max_line_num.max(ln).max(rn);
            rows.push(DiffRow::Pair {
                lang: current_lang.clone(),
                left_line: Some(ln),
                right_line: Some(rn),
                left_text: content.to_owned(),
                right_text: content.to_owned(),
                left_emph: Vec::new(),
                right_emph: Vec::new(),
                kind: PairKind::Context,
            });
        }
    }
    flush_pending(
        &mut rows,
        &mut pending_removed,
        &mut pending_added,
        &current_lang,
    );

    // For a single-file diff, drop redundant top FileHeader and a single starting @@ -1,... @@ header
    // when showing the full file from line 1, matching Rider's clean full-file view.
    let multi_file = file_count > 1;
    if !multi_file {
        if matches!(rows.first(), Some(DiffRow::FileHeader { .. })) {
            rows.remove(0);
        }
        if hunk_count == 1
            && let Some(DiffRow::HunkHeader { header }) = rows.first()
            && (header.starts_with("@@ -1,")
                || header.starts_with("@@ -0,0 +1,")
                || header.starts_with("@@ -1 +"))
        {
            rows.remove(0);
        }
    }

    // Index contiguous change blocks for ↑ / ↓ navigation and Overview Ruler.
    let mut change_blocks = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if let DiffRow::Pair { kind, .. } = rows[i]
            && kind != PairKind::Context
        {
            let start = i;
            let mut has_added = false;
            let mut has_removed = false;
            let mut has_modified = false;
            while i < rows.len() {
                if let DiffRow::Pair { kind: k, .. } = rows[i]
                    && k != PairKind::Context
                {
                    match k {
                        PairKind::Added => has_added = true,
                        PairKind::Removed => has_removed = true,
                        PairKind::Modified => has_modified = true,
                        PairKind::Context => {}
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            let block_kind = if has_modified || (has_added && has_removed) {
                PairKind::Modified
            } else if has_added {
                PairKind::Added
            } else {
                PairKind::Removed
            };
            change_blocks.push(ChangeBlock {
                start_row: start,
                end_row: i,
                kind: block_kind,
            });
        } else {
            i += 1;
        }
    }

    // Check whether there is any long context span worth offering "Collapse unchanged".
    let mut has_collapsible_context = false;
    let mut context_run = 0usize;
    for row in &rows {
        if matches!(
            row,
            DiffRow::Pair {
                kind: PairKind::Context,
                ..
            }
        ) {
            context_run += 1;
            if context_run >= FOLD_KEEP_CONTEXT * 2 + FOLD_MIN_SKIP {
                has_collapsible_context = true;
                break;
            }
        } else {
            context_run = 0;
        }
    }

    ParsedDiff {
        rows,
        change_blocks,
        max_line_num,
        has_collapsible_context,
        multi_file,
    }
}

fn extract_git_diff_path(rest: &str) -> String {
    // Format: "a/path/to/file b/path/to/file"
    if let Some((_, b_part)) = rest.rsplit_once(" b/") {
        return b_part.trim_matches('"').to_owned();
    }
    rest.split_whitespace()
        .last()
        .unwrap_or_default()
        .trim_matches('"')
        .strip_prefix("b/")
        .unwrap_or_default()
        .to_owned()
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    // Example: "@@ -12,7 +15,9 @@ fn foo()"
    let after = line.strip_prefix("@@ ")?;
    let (range_part, _) = after.split_once(" @@")?;
    let mut parts = range_part.split_whitespace();
    let old_spec = parts.next()?.strip_prefix('-')?;
    let new_spec = parts.next()?.strip_prefix('+')?;
    let old_start = old_spec.split(',').next()?.parse::<usize>().ok()?;
    let new_start = new_spec.split(',').next()?.parse::<usize>().ok()?;
    Some((old_start.max(1), new_start.max(1)))
}

/// Tokenize a single line into word / whitespace / punctuation byte ranges for inline diffing.
fn tokenize_words(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let mut end = start + ch.len_utf8();
        if ch.is_alphanumeric() || ch == '_' {
            while let Some(&(i, next_ch)) = chars.peek() {
                if next_ch.is_alphanumeric() || next_ch == '_' {
                    end = i + next_ch.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        } else if ch.is_whitespace() {
            while let Some(&(i, next_ch)) = chars.peek() {
                if next_ch.is_whitespace() {
                    end = i + next_ch.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        }
        out.push(start..end);
    }
    out
}

/// Compute highlighted changed byte ranges for a modified line pair.
fn compute_inline_diff(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old.is_empty() || new.is_empty() || old == new {
        return (Vec::new(), Vec::new());
    }
    let old_toks = tokenize_words(old);
    let new_toks = tokenize_words(new);

    let mut prefix = 0usize;
    let min_len = old_toks.len().min(new_toks.len());
    while prefix < min_len && old[old_toks[prefix].clone()] == new[new_toks[prefix].clone()] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < (min_len - prefix)
        && old[old_toks[old_toks.len() - 1 - suffix].clone()]
            == new[new_toks[new_toks.len() - 1 - suffix].clone()]
    {
        suffix += 1;
    }

    let o_mid = &old_toks[prefix..(old_toks.len() - suffix)];
    let n_mid = &new_toks[prefix..(new_toks.len() - suffix)];
    if o_mid.is_empty() && n_mid.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let (mut o_ranges, mut n_ranges) = if o_mid.len() * n_mid.len() <= MAX_INLINE_DIFF_CELLS
        && !o_mid.is_empty()
        && !n_mid.is_empty()
    {
        // Word-level LCS on the changed middle slice.
        let m = o_mid.len();
        let n = n_mid.len();
        let mut dp = vec![0u16; (m + 1) * (n + 1)];
        let stride = n + 1;
        for i in (0..m).rev() {
            let oi = &old[o_mid[i].clone()];
            for j in (0..n).rev() {
                dp[i * stride + j] = if oi == &new[n_mid[j].clone()] {
                    dp[(i + 1) * stride + (j + 1)] + 1
                } else {
                    dp[(i + 1) * stride + j].max(dp[i * stride + (j + 1)])
                };
            }
        }
        let mut o_changed = Vec::new();
        let mut n_changed = Vec::new();
        let (mut i, mut j) = (0usize, 0usize);
        while i < m && j < n {
            if old[o_mid[i].clone()] == new[n_mid[j].clone()] {
                i += 1;
                j += 1;
            } else if dp[(i + 1) * stride + j] >= dp[i * stride + (j + 1)] {
                push_merged(&mut o_changed, o_mid[i].clone());
                i += 1;
            } else {
                push_merged(&mut n_changed, n_mid[j].clone());
                j += 1;
            }
        }
        while i < m {
            push_merged(&mut o_changed, o_mid[i].clone());
            i += 1;
        }
        while j < n {
            push_merged(&mut n_changed, n_mid[j].clone());
            j += 1;
        }
        (o_changed, n_changed)
    } else {
        let mut o_changed = Vec::new();
        let mut n_changed = Vec::new();
        if let (Some(first), Some(last)) = (o_mid.first(), o_mid.last()) {
            o_changed.push(first.start..last.end);
        }
        if let (Some(first), Some(last)) = (n_mid.first(), n_mid.last()) {
            n_changed.push(first.start..last.end);
        }
        (o_changed, n_changed)
    };

    // Suppress noisy full-line emphasis when nearly the entire line changed.
    let o_changed_bytes: usize = o_ranges.iter().map(|r| r.len()).sum();
    let n_changed_bytes: usize = n_ranges.iter().map(|r| r.len()).sum();
    if o_changed_bytes * 100 >= old.len() * 88 && n_changed_bytes * 100 >= new.len() * 88 {
        o_ranges.clear();
        n_ranges.clear();
    }
    (o_ranges, n_ranges)
}

fn push_merged(ranges: &mut Vec<Range<usize>>, next: Range<usize>) {
    if let Some(last) = ranges.last_mut()
        && last.end == next.start
    {
        last.end = next.end;
        return;
    }
    ranges.push(next);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paired_modifications_and_inline_word_diffs() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,5 @@
 fn greet() {
-    println!(\"hello world\");
+    println!(\"hello naro\");
+    let count = 1;
 }
";
        let parsed = parse_unified_diff(diff, "src/lib.rs");
        assert!(!parsed.multi_file);
        assert_eq!(parsed.change_blocks.len(), 1);
        assert_eq!(parsed.rows.len(), 4);

        match &parsed.rows[1] {
            DiffRow::Pair {
                kind,
                left_line,
                right_line,
                left_text,
                right_text,
                left_emph,
                right_emph,
                ..
            } => {
                assert!(*kind == PairKind::Modified);
                assert_eq!(*left_line, Some(2));
                assert_eq!(*right_line, Some(2));
                assert_eq!(left_emph.len(), 1);
                assert_eq!(right_emph.len(), 1);
                assert_eq!(&left_text[left_emph[0].clone()], "world");
                assert_eq!(&right_text[right_emph[0].clone()], "naro");
            }
            other => panic!("expected modified pair, got {other:?}"),
        }
        assert!(matches!(
            parsed.rows[2],
            DiffRow::Pair {
                kind: PairKind::Added,
                left_line: None,
                right_line: Some(3),
                ..
            }
        ));
    }

    #[test]
    fn folds_long_unchanged_context_and_respects_expanded_folds() {
        let mut diff = String::from(
            "--- a/demo.rs\n+++ b/demo.rs\n@@ -1,22 +1,22 @@\n-old_first();\n+new_first();\n",
        );
        for i in 2..=21 {
            diff.push_str(&format!(" let line_{i} = {i};\n"));
        }
        diff.push_str("-old_last();\n+new_last();\n");

        let parsed = parse_unified_diff(&diff, "demo.rs");
        assert!(parsed.has_collapsible_context);
        assert_eq!(parsed.change_blocks.len(), 2);

        let collapsed = build_side_by_side_items(&parsed, true, &HashSet::new());
        let fold = collapsed.iter().find_map(|item| match item {
            DisplayItem::Folded {
                fold_id,
                skipped_lines,
            } => Some((*fold_id, *skipped_lines)),
            _ => None,
        });
        let (fold_id, skipped) = fold.expect("expected folded context span");
        assert_eq!(skipped, 14); // 20 unchanged lines minus 3 top + 3 bottom context

        let mut expanded = HashSet::new();
        expanded.insert(fold_id);
        let after_expand = build_side_by_side_items(&parsed, true, &expanded);
        assert_eq!(after_expand.len(), parsed.rows.len());
    }
}
