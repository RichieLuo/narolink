use crate::engine::attachments::ImageAttachment;
use eframe::{
    EframeWinitApplication, UserEvent,
    egui::{
        self, Button, Id, Key as EguiKey, KeyboardShortcut, Modifiers, PointerButton, Response,
        TextEdit, Ui, ViewportCommand, text_edit::TextEditOutput,
    },
};
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, StartCause, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
    window::WindowId,
};

type ClipboardImage = Result<ImageAttachment, String>;
pub(super) const CLIPBOARD_IMAGE_NAME: &str = "剪贴板图片.png";

#[derive(Default)]
pub(super) struct ComposerClipboard {
    ctx: OnceCell<egui::Context>,
    pub(super) editor: Cell<Option<egui::Id>>,
    images: RefCell<Vec<(egui::Id, ClipboardImage)>>,
}

impl ComposerClipboard {
    pub(super) fn set_context(&self, ctx: egui::Context) {
        let _ = self.ctx.set(ctx);
    }

    pub(super) fn initialized(&self) -> bool {
        self.ctx.get().is_some()
    }

    pub(super) fn take_images(
        &self,
        editor: egui::Id,
    ) -> impl Iterator<Item = ClipboardImage> + use<> {
        self.images
            .take()
            .into_iter()
            .filter_map(move |(target, image)| (target == editor).then_some(image))
    }

    fn paste(&self) -> bool {
        let (Some(ctx), Some(editor)) = (self.ctx.get(), self.editor.get()) else {
            return false;
        };
        if ctx.memory(|memory| memory.has_focus(editor))
            && let Some(image) = read_clipboard_image()
        {
            self.images.borrow_mut().push((editor, image));
            ctx.request_repaint();
            return true;
        }
        false
    }
}

pub(super) fn read_clipboard_image() -> Option<ClipboardImage> {
    #[cfg(target_os = "macos")]
    if let Some(png) = super::macos::clipboard_png() {
        return Some(png);
    }
    let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
    let dimensions = u32::try_from(image.width)
        .and_then(|width| u32::try_from(image.height).map(|height| (width, height)));
    Some(
        dimensions
            .map_err(|_| "图片尺寸超出限制。".into())
            .and_then(|(width, height)| {
                ImageAttachment::from_rgba(CLIPBOARD_IMAGE_NAME, width, height, &image.bytes)
                    .map_err(|error| error.to_string())
            }),
    )
}

pub(super) fn text_edit(ui: &mut Ui, edit: TextEdit<'_>, password: bool) -> Response {
    text_edit_with_menu(ui, ui.next_auto_id(), edit, password, || false).response
}

pub(super) fn text_edit_with_menu(
    ui: &mut Ui,
    id: Id,
    edit: TextEdit<'_>,
    password: bool,
    handle_paste: impl FnOnce() -> bool,
) -> TextEditOutput {
    let right_press = ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary));
    let selection = right_press
        .then(|| TextEdit::load_state(ui.ctx(), id)?.cursor.char_range())
        .flatten();
    let mut editor = edit.id(id).password(password).show(ui);
    let response = &editor.response;
    // TextEdit moves the cursor on mouse down, including when opening the menu.
    if let Some(range) = selection
        && response.hovered()
        && !range.is_empty()
    {
        editor.state.cursor.set_char_range(Some(range));
        editor.state.clone().store(ui.ctx(), id);
    }

    response.context_menu(|ui| {
        let language = super::Language::in_ui(ui);
        ui.set_width(156.);
        let is_mac = ui.ctx().os().is_mac();
        let modifiers = if is_mac {
            egui::ModifierNames::SYMBOLS
        } else {
            egui::ModifierNames::NAMES
        };
        let item = |label, key| {
            let shortcut = KeyboardShortcut::new(Modifiers::COMMAND, key);
            Button::new(language.text(label)).shortcut_text(shortcut.format(&modifiers, is_mac))
        };
        let selected = editor
            .state
            .cursor
            .char_range()
            .is_some_and(|range| !range.is_empty());
        for (label, key, command) in [
            ("剪切", EguiKey::X, ViewportCommand::RequestCut),
            ("复制", EguiKey::C, ViewportCommand::RequestCopy),
        ] {
            if ui
                .add_enabled(selected && !password, item(label, key))
                .clicked()
            {
                response.request_focus();
                ui.ctx().send_viewport_cmd(command);
                ui.close();
            }
        }
        if ui.add(item("粘贴", EguiKey::V)).clicked() {
            response.request_focus();
            if !handle_paste() {
                ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
            }
            ui.close();
        }
        if ui
            .add_enabled(!editor.galley.is_empty(), item("全选", EguiKey::A))
            .clicked()
        {
            response.request_focus();
            let range = egui::text::CCursorRange::select_all(&editor.galley);
            editor.state.cursor.set_char_range(Some(range));
            editor.state.clone().store(ui.ctx(), id);
            ui.close();
        }
    });
    editor
}

// egui-winit consumes the paste keystroke before it reaches egui and only reads text.
pub(super) struct NativeApp<'a> {
    app: EframeWinitApplication<'a>,
    clipboard: Rc<ComposerClipboard>,
    modifiers: ModifiersState,
}

impl<'a> NativeApp<'a> {
    pub(super) fn new(app: EframeWinitApplication<'a>, clipboard: Rc<ComposerClipboard>) -> Self {
        Self {
            app,
            clipboard,
            modifiers: ModifiersState::empty(),
        }
    }
}

impl ApplicationHandler<UserEvent> for NativeApp<'_> {
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) => self.modifiers = ModifiersState::empty(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } if event.state == ElementState::Pressed
                && is_paste(&event.logical_key, event.physical_key, self.modifiers)
                && self.clipboard.paste() =>
            {
                return;
            }
            _ => {}
        }
        self.app.window_event(event_loop, id, event);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.app.new_events(event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.app.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.app.suspended(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.app.user_event(event_loop, event);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.app.device_event(event_loop, id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.app.about_to_wait(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.app.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.app.memory_warning(event_loop);
    }
}

fn is_paste(key: &Key, physical_key: PhysicalKey, modifiers: ModifiersState) -> bool {
    if *key == Key::Named(NamedKey::Paste) {
        return true;
    }
    let command = if cfg!(target_os = "macos") {
        ModifiersState::SUPER
    } else {
        ModifiersState::CONTROL
    };
    let is_v = match key.as_ref() {
        Key::Character(value) if value.is_ascii() => value.eq_ignore_ascii_case("v"),
        _ => physical_key == PhysicalKey::Code(KeyCode::KeyV),
    };
    (modifiers == command && is_v)
        || (cfg!(target_os = "windows")
            && modifiers == ModifiersState::SHIFT
            && *key == Key::Named(NamedKey::Insert))
}

pub(super) fn send_shortcut(events: &mut Vec<egui::Event>, composing: &mut bool) -> bool {
    use egui::{Event, ImeEvent, Key};
    let mut ime_used = *composing;
    for event in events.iter() {
        match event {
            Event::Ime(ImeEvent::Preedit(text)) => {
                *composing = !text.is_empty();
                ime_used |= *composing;
            }
            Event::Ime(ImeEvent::Commit(_)) => {
                *composing = false;
                ime_used = true;
            }
            Event::Ime(ImeEvent::Disabled) | Event::WindowFocused(false) => *composing = false,
            _ => {}
        }
    }
    let mut send = false;
    events.retain(|event| {
        if let Event::Key {
            key: Key::Enter,
            pressed: true,
            repeat,
            modifiers,
            ..
        } = event
        {
            send |= !ime_used && !repeat && modifiers.is_none();
            return !ime_used && *modifiers == egui::Modifiers::SHIFT;
        }
        true
    });
    send
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, ImeEvent, Key, Modifiers};

    fn enter(modifiers: Modifiers, repeat: bool) -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }

    #[test]
    fn composer_return_distinguishes_send_newline_and_ime_confirmation() {
        let mut composing = false;
        let mut preedit = vec![Event::Ime(ImeEvent::Preedit("nihao".into()))];
        assert!(!send_shortcut(&mut preedit, &mut composing));
        assert!(composing);

        let mut commit = vec![
            Event::Ime(ImeEvent::Commit("你好，".into())),
            enter(Modifiers::NONE, false),
        ];
        assert!(!send_shortcut(&mut commit, &mut composing));
        assert!(!composing);
        assert_eq!(commit, [Event::Ime(ImeEvent::Commit("你好，".into()))]);

        let newline = enter(Modifiers::SHIFT, false);
        let mut events = vec![newline.clone(), enter(Modifiers::NONE, true)];
        assert!(!send_shortcut(&mut events, &mut composing));
        assert_eq!(events, [newline]);

        let mut events = vec![enter(Modifiers::NONE, false)];
        assert!(send_shortcut(&mut events, &mut composing));
        assert!(events.is_empty());
    }
}
