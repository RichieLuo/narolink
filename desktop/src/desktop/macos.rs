use crate::engine::attachments::{ImageAttachment, MAX_IMAGE_SOURCE_BYTES};
use objc2::{
    MainThreadMarker, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSEventModifierFlags, NSEventType, NSMarkedClauseSegmentAttributeName,
    NSModalResponseOK, NSOpenPanel, NSPasteboard, NSPasteboardTypePNG,
    NSUnderlineStyleAttributeName,
};
use objc2_foundation::{NSArray, NSAttributedString, NSRange, NSString, NSURL};
use std::path::{Path, PathBuf};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) fn pick_attachments(directory: &Path, title: &str, add: &str) -> Option<Vec<PathBuf>> {
    let mtm = MainThreadMarker::new()?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setTitle(Some(&NSString::from_str(title)));
    panel.setPrompt(Some(&NSString::from_str(add)));
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(true);
    if let Some(directory) = directory.to_str().filter(|_| directory.is_dir()) {
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath_isDirectory(
            &NSString::from_str(directory),
            true,
        )));
    }
    (panel.runModal() == NSModalResponseOK).then(|| {
        panel
            .URLs()
            .iter()
            .filter_map(|url| url.path().map(|path| PathBuf::from(path.to_string())))
            .collect()
    })
}

pub(super) fn clipboard_png() -> Option<Result<ImageAttachment, String>> {
    autoreleasepool(|_| {
        // arboard's macOS reader only requests TIFF. Screenshots can offer PNG directly.
        let data = NSPasteboard::generalPasteboard().dataForType(unsafe { NSPasteboardTypePNG })?;
        Some(if data.len() > MAX_IMAGE_SOURCE_BYTES {
            Err("图片总大小不能超过 12 MiB。".into())
        } else {
            ImageAttachment::from_bytes(super::input::CLIPBOARD_IMAGE_NAME, &data.to_vec())
                .map_err(|error| error.to_string())
        })
    })
}

// winit 0.30 discards AppKit's insertText calls without marked text. Some input
// methods use such calls for full-width punctuation. Hook only Naro's own view.
pub(super) fn install_ime_workaround(cc: &eframe::CreationContext<'_>) {
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: eframe provides a live NSView on the AppKit thread.
    let view = unsafe { &*handle.ns_view.cast::<AnyObject>().as_ptr() };
    let superclass = view.class();
    if superclass.name() != c"WinitView" {
        return;
    }
    let Some(mut class) = ClassBuilder::new(c"NaroInputView", superclass) else {
        return;
    };
    // SAFETY: Both methods use the NSTextInputClient signatures. Adding no ivars
    // keeps the subclass layout identical to WinitView's.
    unsafe {
        class.add_method(
            sel!(insertText:replacementRange:),
            insert_text as extern "C-unwind" fn(_, _, _, _),
        );
        class.add_method(
            sel!(validAttributesForMarkedText),
            marked_text_attributes as extern "C-unwind" fn(_, _) -> _,
        );
        AnyObject::set_class(view, class.register());
    }
}

extern "C-unwind" fn marked_text_attributes(_: &AnyObject, _: Sel) -> *mut NSArray<NSString> {
    let attributes = unsafe {
        [
            NSUnderlineStyleAttributeName,
            NSMarkedClauseSegmentAttributeName,
        ]
    };
    Retained::autorelease_return(NSArray::from_slice(&attributes))
}

extern "C-unwind" fn insert_text(
    view: &AnyObject,
    _: Sel,
    value: &AnyObject,
    replacement: NSRange,
) {
    let superclass = AnyClass::get(c"WinitView").expect("NaroInputView base class");
    // SAFETY: The subclass is installed only on WinitView, which implements NSTextInputClient.
    unsafe {
        let marked: Bool = msg_send![view, hasMarkedText];
        if !marked.as_bool()
            && let Some(length) = translated_key_text(value)
        {
            // Give winit a mark and its commit in the same key event. It then delivers
            // the IME's text once and consumes the untranslated keystroke.
            let _: () = msg_send![view, setMarkedText: value, selectedRange: NSRange::new(length, 0), replacementRange: replacement];
        }
        let _: () =
            msg_send![super(view, superclass), insertText: value, replacementRange: replacement];
    }
}

fn translated_key_text(value: &AnyObject) -> Option<usize> {
    let mtm = MainThreadMarker::new()?;
    let event = NSApplication::sharedApplication(mtm).currentEvent()?;
    if event.r#type() != NSEventType::KeyDown
        || event
            .modifierFlags()
            .intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control)
    {
        return None;
    }
    let attributed = value
        .downcast_ref::<NSAttributedString>()
        .map(NSAttributedString::string);
    let text = attributed
        .as_deref()
        .or_else(|| value.downcast_ref::<NSString>())?;
    let raw = event.characters()?;
    if text.length() == 0
        || text.isEqualToString(&raw)
        || text.to_string().chars().any(char::is_control)
    {
        return None;
    }
    Some(text.length())
}
