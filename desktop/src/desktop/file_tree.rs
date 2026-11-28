//! Desktop workspace directory tree, shown as a right-hand split panel.
use super::{
    Language, git::status_color, markdown,
    style::{Icon, Palette},
};
use crate::git::GitStatus;
use eframe::egui::{
    self, Align, Align2, FontId, Label, Layout, Margin, Rect, RichText, Sense, Stroke, pos2, vec2,
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const ROW_HEIGHT: f32 = 24.;
const INDENT: f32 = 14.;
const MAX_ENTRIES: usize = 5000;

pub(super) enum TreeCommand {
    Close,
    Refresh,
    OpenFile(PathBuf),
}

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    name: String,
    dir: bool,
}

enum Listing {
    Entries {
        items: Arc<Vec<Entry>>,
        truncated: bool,
    },
    Failed(&'static str),
}

enum Row {
    Entry {
        entry: Entry,
        depth: usize,
        expanded: bool,
    },
    Note {
        depth: usize,
        text: &'static str,
    },
}

#[derive(Default)]
struct Marks {
    files: HashMap<PathBuf, String>,
    dirs: HashSet<PathBuf>,
}

#[derive(Default)]
pub(super) struct FileTree {
    open: bool,
    root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Listing>,
    selected: Option<PathBuf>,
    marks: Marks,
    marks_key: Option<(u64, bool)>,
}

impl FileTree {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn open(&mut self) {
        self.open = true;
    }

    pub(super) fn close(&mut self) {
        self.open = false;
    }

    /// Returns true when the root changed. Expanded folders and listings are
    /// tied to the previous root, so they are dropped with it.
    pub(super) fn set_root(&mut self, root: Option<PathBuf>) -> bool {
        if self.root == root {
            return false;
        }
        *self = Self {
            open: self.open,
            root,
            ..Self::default()
        };
        true
    }

    /// Forgets cached listings so folders are read from disk again.
    pub(super) fn refresh(&mut self) {
        self.listings.clear();
    }

    fn ensure(&mut self, dir: &Path) {
        if !self.listings.contains_key(dir) {
            self.listings.insert(dir.to_owned(), read_directory(dir));
        }
    }

    fn rows(&mut self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(root) = self.root.clone() {
            self.collect(&root, 0, &mut rows);
        }
        rows
    }

    fn collect(&mut self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        self.ensure(dir);
        let (items, truncated) = match self.listings.get(dir) {
            Some(Listing::Entries { items, truncated }) => (items.clone(), *truncated),
            Some(Listing::Failed(text)) => {
                rows.push(Row::Note { depth, text });
                return;
            }
            None => return,
        };
        if items.is_empty() {
            rows.push(Row::Note {
                depth,
                text: "空文件夹",
            });
        }
        for entry in items.iter() {
            let expanded = entry.dir && self.expanded.contains(&entry.path);
            rows.push(Row::Entry {
                entry: entry.clone(),
                depth,
                expanded,
            });
            if expanded {
                self.collect(&entry.path, depth + 1, rows);
            }
        }
        if truncated {
            rows.push(Row::Note {
                depth,
                text: "文件过多，仅显示前 5000 项",
            });
        }
    }

    fn sync_marks(&mut self, status: Option<&GitStatus>, serial: u64) {
        let key = (serial, status.is_some());
        if self.marks_key == Some(key) {
            return;
        }
        self.marks_key = Some(key);
        self.marks = match (&self.root, status) {
            (Some(root), Some(status)) => build_marks(root, status),
            _ => Marks::default(),
        };
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        status: Option<&GitStatus>,
        serial: u64,
        loading: bool,
    ) -> Option<TreeCommand> {
        if !self.open {
            return None;
        }
        self.sync_marks(status, serial);
        let rows = self.rows();
        let palette = Palette::from_context(ctx);
        let available = ctx.available_rect().width();
        let chat_width = (available * 0.46).clamp(360., 440.);
        let max_width = (available - chat_width).max(280.);
        let mut command = None;
        let mut toggle = None;
        let mut select = None;
        let marks = &self.marks;
        let selected = &self.selected;
        let root = self.root.clone();

        egui::SidePanel::right("file tree split")
            .default_width((available * 0.34).clamp(280., 420.))
            .width_range(260. ..=max_width)
            .resizable(true)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(palette.paper))
            .show(ctx, |ui| {
                // Keep overflowing content from changing the persisted panel width.
                ui.take_available_width();
                let ui = &mut ui.new_child(egui::UiBuilder::new());
                let language = Language::in_ui(ui);
                let bounds = ui.max_rect();
                ui.spacing_mut().item_spacing.y = 0.;
                ui.painter().vline(
                    bounds.left() + 0.5,
                    bounds.y_range(),
                    Stroke::new(1., palette.line),
                );
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
                                    .button(ui, language.text("关闭文件目录"), palette.muted)
                                    .clicked()
                                {
                                    command = Some(TreeCommand::Close);
                                }
                                if Icon::refresh_button(
                                    ui,
                                    language.text("刷新文件目录"),
                                    palette.muted,
                                    loading,
                                )
                                .clicked()
                                {
                                    command = Some(TreeCommand::Refresh);
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 7.;
                                    let (rect, _) = ui
                                        .allocate_exact_size(vec2(16., 16.), Sense::hover());
                                    Icon::Folder.paint(ui.painter(), rect.center(), palette.accent);
                                    let name = root
                                        .as_deref()
                                        .and_then(Path::file_name)
                                        .map(|name| name.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| language.text("文件目录").to_owned());
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
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top() - 0.5,
                    Stroke::new(1., palette.line),
                );
                if root.is_none() {
                    ui.add_space(24.);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(language.text("请先选择工作目录"))
                                .size(12.5)
                                .color(palette.muted),
                        );
                    });
                    return;
                }
                ui.add_space(4.);
                egui::ScrollArea::vertical()
                    .id_salt("file tree rows")
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                        for row in &rows[range] {
                            match row {
                                Row::Note { depth, text } => {
                                    let (rect, _) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), ROW_HEIGHT),
                                        Sense::hover(),
                                    );
                                    ui.painter().text(
                                        pos2(row_left(rect, *depth) + 30., rect.center().y),
                                        Align2::LEFT_CENTER,
                                        language.text(text),
                                        FontId::proportional(12.),
                                        palette.muted,
                                    );
                                }
                                Row::Entry {
                                    entry,
                                    depth,
                                    expanded,
                                } => {
                                    let (rect, response) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), ROW_HEIGHT),
                                        Sense::click(),
                                    );
                                    let mark = marks.files.get(&entry.path);
                                    let changed_dir = entry.dir && marks.dirs.contains(&entry.path);
                                    let highlight = selected.as_ref() == Some(&entry.path);
                                    let painter = ui.painter();
                                    if highlight || response.hovered() {
                                        painter.rect_filled(
                                            rect.shrink2(vec2(6., 1.)),
                                            5,
                                            if highlight { palette.tint } else { palette.soft },
                                        );
                                    }
                                    let left = row_left(rect, *depth);
                                    if entry.dir {
                                        let chevron = if *expanded { Icon::Down } else { Icon::Right };
                                        chevron.paint(
                                            painter,
                                            pos2(left + 7., rect.center().y),
                                            palette.muted,
                                        );
                                    }
                                    let icon = if entry.dir { Icon::Folder } else { Icon::Document };
                                    icon.paint(
                                        painter,
                                        pos2(left + 28., rect.center().y),
                                        if entry.dir { palette.accent } else { palette.muted },
                                    );
                                    let color = mark.map_or(palette.ink, |mark| {
                                        status_color(&mark[..1.min(mark.len())], palette)
                                    });
                                    let badge_width = 26.;
                                    let text_clip = Rect::from_min_max(
                                        pos2(left + 40., rect.top()),
                                        pos2(rect.right() - badge_width, rect.bottom()),
                                    );
                                    painter.with_clip_rect(text_clip).text(
                                        pos2(left + 40., rect.center().y),
                                        Align2::LEFT_CENTER,
                                        &entry.name,
                                        FontId::proportional(13.),
                                        color,
                                    );
                                    let badge = pos2(rect.right() - 14., rect.center().y);
                                    if let Some(mark) = mark {
                                        let letter = mark.chars().next().unwrap_or('M');
                                        painter.text(
                                            badge,
                                            Align2::CENTER_CENTER,
                                            letter,
                                            FontId::proportional(11.5),
                                            color,
                                        );
                                    } else if changed_dir {
                                        painter.circle_filled(badge, 3., palette.accent);
                                    }
                                    if response.clicked() {
                                        select = Some(entry.path.clone());
                                        if entry.dir {
                                            toggle = Some(entry.path.clone());
                                        } else {
                                            command = Some(TreeCommand::OpenFile(entry.path.clone()));
                                        }
                                    }
                                    response.context_menu(|ui| {
                                        if ui.button(language.text("在文件夹中显示")).clicked() {
                                            let path = entry.path.clone();
                                            std::thread::spawn(move || markdown::reveal_file(&path));
                                            ui.close();
                                        }
                                        if ui.button(language.text("复制路径")).clicked() {
                                            ui.ctx().copy_text(entry.path.display().to_string());
                                            ui.close();
                                        }
                                        if let Some(relative) = root
                                            .as_deref()
                                            .and_then(|root| entry.path.strip_prefix(root).ok())
                                            && ui.button(language.text("复制相对路径")).clicked()
                                        {
                                            ui.ctx().copy_text(relative.display().to_string());
                                            ui.close();
                                        }
                                    });
                                }
                            }
                        }
                    });
            });

        if let Some(path) = select {
            self.selected = Some(path);
        }
        if let Some(path) = toggle
            && !self.expanded.remove(&path)
        {
            self.expanded.insert(path);
        }
        command
    }
}

fn row_left(rect: Rect, depth: usize) -> f32 {
    rect.left() + 10. + depth as f32 * INDENT
}

fn read_directory(dir: &Path) -> Listing {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            return Listing::Failed(if error.kind() == std::io::ErrorKind::PermissionDenied {
                "没有权限读取此文件夹。"
            } else {
                "无法读取此文件夹。"
            });
        }
    };
    let mut items = Vec::new();
    let mut truncated = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Git's own metadata is never useful in a project browser.
        if name == ".git" {
            continue;
        }
        if items.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let path = entry.path();
        // Symlinks are followed so links to folders can be expanded.
        let dir = entry
            .file_type()
            .map(|kind| {
                kind.is_dir() || (kind.is_symlink() && fs::metadata(&path).is_ok_and(|m| m.is_dir()))
            })
            .unwrap_or(false);
        items.push(Entry { path, name, dir });
    }
    items.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Listing::Entries {
        items: Arc::new(items),
        truncated,
    }
}

/// Maps Git changes onto tree paths. Git reports paths relative to the
/// repository root, which can sit above (or resolve differently from) the tree root.
fn build_marks(root: &Path, status: &GitStatus) -> Marks {
    let mut marks = Marks::default();
    let Some(git_root) = status.root.as_deref().map(PathBuf::from) else {
        return marks;
    };
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    // Later groups win, so conflicts and unstaged edits outrank staged and new files.
    let groups = [
        (&status.untracked, None),
        (&status.staged, None),
        (&status.unstaged, None),
        (&status.conflicted, Some("U")),
    ];
    for (changes, forced) in groups {
        for change in changes {
            let Ok(relative) = git_root.join(&change.path).strip_prefix(&canonical).map(Path::to_owned)
            else {
                continue;
            };
            let path = root.join(relative);
            let label = forced.map_or_else(|| change.status.clone(), str::to_owned);
            marks.files.insert(path.clone(), label);
            let mut current = path.as_path();
            while let Some(parent) = current.parent() {
                if !parent.starts_with(root) {
                    break;
                }
                marks.dirs.insert(parent.to_owned());
                if parent == root {
                    break;
                }
                current = parent;
            }
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitFileChange;

    fn change(path: &str, status: &str) -> GitFileChange {
        GitFileChange {
            path: path.into(),
            orig_path: None,
            status: status.into(),
        }
    }

    #[test]
    fn lists_folders_first_and_hides_git_metadata() {
        let dir = tempfile::tempdir().unwrap();
        for folder in [".git", "src", "Docs"] {
            fs::create_dir(dir.path().join(folder)).unwrap();
        }
        for file in ["b.txt", "A.txt", ".env"] {
            fs::write(dir.path().join(file), "").unwrap();
        }
        let Listing::Entries { items, truncated } = read_directory(dir.path()) else {
            panic!("directory should be readable");
        };
        let names: Vec<_> = items.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Docs", "src", ".env", "A.txt", "b.txt"]);
        assert!(!truncated);
    }

    #[test]
    fn reports_unreadable_directories() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_directory(&dir.path().join("missing")),
            Listing::Failed("无法读取此文件夹。")
        ));
    }

    #[test]
    fn expands_folders_and_renders_in_every_language() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::create_dir(dir.path().join("empty")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "").unwrap();
        fs::write(dir.path().join("a.txt"), "").unwrap();
        let mut tree = FileTree::default();
        assert!(tree.set_root(Some(dir.path().to_owned())));
        tree.open();
        assert_eq!(tree.rows().len(), 3);
        tree.expanded.insert(dir.path().join("src"));
        tree.expanded.insert(dir.path().join("empty"));
        // empty folder note + main.rs on top of the three entries
        assert_eq!(tree.rows().len(), 5);
        // A new root forgets state tied to the old one but keeps the panel open.
        assert!(!tree.set_root(Some(dir.path().to_owned())));
        assert!(tree.set_root(None));
        assert!(tree.is_open() && tree.expanded.is_empty());
        tree.set_root(Some(dir.path().to_owned()));
        tree.expanded.insert(dir.path().join("src"));

        for language in Language::ALL {
            let ctx = egui::Context::default();
            super::super::style::setup(&ctx);
            language.apply(&ctx);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1000., 700.))),
                ..Default::default()
            };
            let mut command = None;
            let _ = ctx.run(input, |ctx| {
                command = tree.show(ctx, None, 0, false);
            });
            assert!(command.is_none());
        }
    }

    #[test]
    fn marks_changed_files_and_their_parent_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let status = GitStatus {
            is_repo: true,
            root: Some(root.display().to_string()),
            staged: vec![change("src/a.rs", "A")],
            unstaged: vec![change("src/deep/b.rs", "M")],
            untracked: vec![change("notes.md", "?")],
            conflicted: vec![change("src/a.rs", "UU")],
            ..GitStatus::default()
        };
        let marks = build_marks(&root, &status);
        assert_eq!(marks.files[&root.join("src/a.rs")], "U");
        assert_eq!(marks.files[&root.join("src/deep/b.rs")], "M");
        assert_eq!(marks.files[&root.join("notes.md")], "?");
        assert!(marks.dirs.contains(&root.join("src")));
        assert!(marks.dirs.contains(&root.join("src/deep")));
        assert!(marks.dirs.contains(&root));
        assert!(!marks.dirs.contains(root.parent().unwrap()));
    }

    #[test]
    fn marks_only_the_part_of_a_repository_below_the_tree_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir(repo.join("app")).unwrap();
        let status = GitStatus {
            is_repo: true,
            root: Some(repo.display().to_string()),
            unstaged: vec![change("app/main.rs", "M"), change("other/lib.rs", "M")],
            ..GitStatus::default()
        };
        let marks = build_marks(&repo.join("app"), &status);
        assert_eq!(marks.files.len(), 1);
        assert_eq!(marks.files[&repo.join("app/main.rs")], "M");
    }
}
