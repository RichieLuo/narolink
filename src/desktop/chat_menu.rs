//! Right-click menu for the conversation panel: copy selected text and file link actions.

use super::{Language, markdown};
use eframe::egui::{
    self, Align, Button, Id, KeyboardShortcut, Layout, Modifiers, PointerButton, Popup,
    PopupAnchor, PopupKind, Rect, SetOpenCommand, Ui, ViewportCommand,
    containers::menu::menu_style, text_selection::LabelSelectionState,
};
use std::path::{Path, PathBuf};

/// Written by the patched `egui_commonmark_backend` while a hooked link is hovered.
const HOVERED_LINK: &str = "egui_commonmark_backend hovered link";
const HOVERED_FILE: &str = "chat menu hovered file";
const MENU: &str = "chat context menu";

#[derive(Clone)]
struct Menu {
    selection: LabelSelectionState,
    file: Option<PathBuf>,
}

/// Remember which local file link, if any, is under the pointer in this frame.
pub(super) fn publish_hovered_file<'a>(
    ctx: &egui::Context,
    mut files: impl Iterator<Item = (&'a str, &'a Path)>,
) {
    let link = ctx.data(|data| data.get_temp::<String>(Id::new(HOVERED_LINK)));
    if let Some(link) = link
        && let Some((_, path)) = files.find(|(target, _)| *target == link)
    {
        ctx.data_mut(|data| data.insert_temp(Id::new(HOVERED_FILE), path.to_owned()));
    }
}

/// Call before drawing the conversation. Returns the label selection as it was before a
/// right press, because pressing any button collapses egui's label selection.
pub(super) fn begin(ui: &Ui, area: Rect) -> Option<LabelSelectionState> {
    let ctx = ui.ctx();
    let restore =
        ctx.data_mut(|data| data.remove_temp::<LabelSelectionState>(Id::new(MENU).with("restore")));
    if let Some(selection) = restore {
        *ctx.plugin::<LabelSelectionState>().lock() = selection;
    }
    ctx.data_mut(|data| {
        data.remove::<String>(Id::new(HOVERED_LINK));
        data.remove::<PathBuf>(Id::new(HOVERED_FILE));
    });
    let pos = ctx.input(|input| {
        input
            .pointer
            .button_pressed(PointerButton::Secondary)
            .then(|| input.pointer.interact_pos())
            .flatten()
    })?;
    (area.contains(pos) && ctx.layer_id_at(pos) == Some(ui.layer_id()))
        .then(|| ctx.plugin::<LabelSelectionState>().lock().clone())
}

/// Call after drawing the conversation. Returns a local file the user asked to open.
pub(super) fn show(ui: &Ui, press: Option<LabelSelectionState>) -> Option<PathBuf> {
    let ctx = ui.ctx();
    let id = Id::new(MENU);
    let pending = id.with("pending");
    if let Some(selection) = press {
        // Keep the highlight and make the selection available to the menu.
        *ctx.plugin::<LabelSelectionState>().lock() = selection.clone();
        let file = ctx.data(|data| data.get_temp::<PathBuf>(Id::new(HOVERED_FILE)));
        ctx.data_mut(|data| {
            data.insert_temp(id.with("state"), Menu { selection, file });
            data.insert_temp(pending, true);
        });
    }
    // Open on release like egui's own context menus: the release of the opening click
    // must not count as a click that dismisses the freshly opened menu.
    let released = ctx.input(|input| input.pointer.button_released(PointerButton::Secondary));
    let open = (released
        && ctx
            .data(|data| data.get_temp::<bool>(pending))
            .unwrap_or(false))
    .then(|| {
        ctx.data_mut(|data| data.remove::<bool>(pending));
        SetOpenCommand::Bool(true)
    });
    let menu = ctx.data(|data| data.get_temp::<Menu>(id.with("state")))?;
    let mut action = None;
    Popup::new(id, ctx.clone(), PopupAnchor::PointerFixed, ui.layer_id())
        .kind(PopupKind::Menu)
        .layout(Layout::top_down_justified(Align::Min))
        .style(menu_style)
        .gap(0.)
        .open_memory(open)
        .show(|ui| {
            let language = Language::in_ui(ui);
            ui.set_min_width(156.);
            let is_mac = ui.ctx().os().is_mac();
            let modifiers = if is_mac {
                egui::ModifierNames::SYMBOLS
            } else {
                egui::ModifierNames::NAMES
            };
            let shortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::C);
            let copy = Button::new(language.text("复制"))
                .shortcut_text(shortcut.format(&modifiers, is_mac));
            if ui
                .add_enabled(menu.selection.has_selection(), copy)
                .clicked()
            {
                // Pressing the menu item cleared the live selection at the end of this pass.
                // Put it back before labels handle the copy request in the next frame.
                ui.ctx()
                    .data_mut(|data| data.insert_temp(id.with("restore"), menu.selection.clone()));
                ui.ctx().send_viewport_cmd(ViewportCommand::RequestCopy);
                ui.close();
            }
            if let Some(path) = &menu.file {
                ui.separator();
                if ui.button(language.text("打开文件")).clicked() {
                    action = Some(path.clone());
                    ui.close();
                }
                if ui.button(language.text("在文件夹中显示")).clicked() {
                    let path = path.clone();
                    std::thread::spawn(move || markdown::reveal_file(&path));
                    ui.close();
                }
                if ui.button(language.text("复制路径")).clicked() {
                    ui.ctx().copy_text(path.display().to_string());
                    ui.close();
                }
            }
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_commonmark::CommonMarkCache;

    fn button(pos: egui::Pos2, button: PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn right_click_keeps_selection_and_offers_file_actions() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut opened = None;
        let mut frame = |events: Vec<egui::Event>| {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600., 400.),
                )),
                events,
                ..Default::default()
            };
            input.focused = true;
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let press = begin(ui, ui.max_rect());
                    markdown::render(
                        ui,
                        &mut cache,
                        "Select this text and [open doc](docs/readme.md)",
                        Path::new("/project"),
                    );
                    if let Some(path) = show(ui, press) {
                        opened = Some(path);
                    }
                });
            })
        };
        let find = |output: &egui::FullOutput, needle: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.text() == needle
                            || (needle.is_ascii() && text.galley.text().contains(needle)) =>
                    {
                        Some(text.visual_bounding_rect())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{needle} was not painted"))
        };
        let first = frame(Vec::new());
        let text = find(&first, "Select this");
        let link = find(&first, "open doc").center();

        // Select with the left button.
        let start = text.left_center() + egui::vec2(2., 0.);
        let end = start + egui::vec2(60., 0.);
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![button(start, PointerButton::Primary, true)]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![egui::Event::PointerMoved(end + egui::vec2(10., 0.))]);
        frame(vec![button(end, PointerButton::Primary, false)]);

        // Right click on the file link.
        frame(vec![egui::Event::PointerMoved(link)]);
        frame(vec![button(link, PointerButton::Secondary, true)]);
        let menu = ctx
            .data(|data| data.get_temp::<Menu>(Id::new(MENU).with("state")))
            .expect("right press stores the menu state");
        assert!(menu.selection.has_selection());
        assert_eq!(menu.file, Some(PathBuf::from("/project/docs/readme.md")));
        frame(vec![button(link, PointerButton::Secondary, false)]);
        assert!(Popup::is_id_open(&ctx, Id::new(MENU)));
        let shown = frame(Vec::new());
        let copy = find(&shown, "复制").center();
        let open = find(&shown, "打开文件").center();
        find(&shown, "在文件夹中显示");
        find(&shown, "复制路径");

        // Copy goes through the label selection captured before the right press.
        frame(vec![egui::Event::PointerMoved(copy)]);
        let clicked = frame(vec![
            button(copy, PointerButton::Primary, true),
            button(copy, PointerButton::Primary, false),
        ]);
        assert!(
            clicked
                .viewport_output
                .values()
                .any(|viewport| viewport.commands.contains(&ViewportCommand::RequestCopy))
        );
        let copied = frame(vec![egui::Event::Copy]);
        assert!(copied.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if !text.is_empty() && "Select this text and ".contains(text.as_str()))
        }));

        // Open the file from a fresh menu.
        frame(vec![egui::Event::PointerMoved(link)]);
        frame(vec![button(link, PointerButton::Secondary, true)]);
        frame(vec![button(link, PointerButton::Secondary, false)]);
        frame(vec![egui::Event::PointerMoved(open)]);
        frame(vec![
            button(open, PointerButton::Primary, true),
            button(open, PointerButton::Primary, false),
        ]);
        assert_eq!(opened, Some(PathBuf::from("/project/docs/readme.md")));
    }
}
