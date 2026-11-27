use super::{
    Language,
    style::{Icon, Palette, surface},
};
use eframe::egui::{self, Align, Label, Layout, Margin, RichText, Stroke, Ui, vec2};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_extras::syntax_highlighting::{self, CodeTheme};
use pulldown_cmark::{Alignment, CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fmt::Write,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    time::Duration,
};
use url::Url;

/// Render a document and return a clicked local link to the application's file viewer.
pub(super) fn render(
    ui: &mut Ui,
    cache: &mut CommonMarkCache,
    text: &str,
    cwd: &Path,
) -> Option<PathBuf> {
    render_markdown(ui, cache, text, Some(cwd))
}

pub(super) fn render_remote(ui: &mut Ui, cache: &mut CommonMarkCache, text: &str) {
    render_markdown(ui, cache, text, None);
}

fn parser_options() -> Options {
    // Match the viewer's parser options so source ranges retain their meaning.
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
}

fn web_link(target: &str) -> bool {
    Url::parse(target).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn render_markdown(
    ui: &mut Ui,
    cache: &mut CommonMarkCache,
    text: &str,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let palette = Palette::of(ui);
    cache.link_hooks_clear();
    let mut local_files = Vec::new();
    let mut table_cache = CommonMarkCache::default();
    let mut blocks = Vec::new();
    let mut code: Option<(Range<usize>, CodeBlock)> = None;
    let mut table: Option<TableBlock<'_>> = None;
    for (event, range) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { id, dest_url, .. }) => {
                if let Some(table) = &mut table {
                    table.add_reference(id, &dest_url);
                }
                if web_link(&dest_url) {
                    continue;
                }
                cache.add_link_hook(dest_url.as_ref());
                if table.is_some() {
                    table_cache.add_link_hook(dest_url.as_ref());
                }
                if let Some(path) = cwd.and_then(|cwd| local_file(&dest_url, cwd)) {
                    local_files.push((dest_url, path));
                }
            }
            Event::Start(Tag::Image { id, dest_url, .. }) => {
                if let Some(table) = &mut table {
                    table.add_reference(id, &dest_url);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => code = CodeBlock::start(text, range, kind),
            Event::Text(content) => {
                if let Some((_, block)) = &mut code {
                    block.content.push_str(&content);
                }
            }
            Event::End(TagEnd::CodeBlock) => blocks.extend(
                code.take()
                    .map(|(range, block)| (range, Block::Code(block))),
            ),
            Event::Start(Tag::Table(alignments)) => {
                table = Some(TableBlock {
                    alignments,
                    source: String::new(),
                    references: BTreeMap::new(),
                });
            }
            Event::Start(Tag::TableHead) => {
                if let Some(table) = &mut table {
                    table.add_header(&text[range]);
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(table) = &mut table {
                    table.source.push_str(&text[range]);
                }
            }
            Event::End(TagEnd::Table) => blocks.extend(
                table
                    .take()
                    .map(|table| (range, Block::Table(table.finish()))),
            ),
            _ => {}
        }
    }

    let table_link = Rc::new(Cell::new(None));
    ui.scope(|ui| {
        // CommonMark builds a LayoutJob for links. An explicit text color on its
        // sections would override the hyperlink color chosen by egui.
        ui.visuals_mut().override_text_color = None;
        ui.visuals_mut().widgets.noninteractive.fg_stroke.color = palette.ink;
        ui.visuals_mut().code_bg_color = palette.soft;
        ui.style_mut().url_in_tooltip = true;
        let viewer = CommonMarkViewer::new().explicit_image_uri_scheme(true);
        let remote = cwd.is_none();
        if blocks.is_empty() {
            show_markdown(ui, cache, text, viewer, remote);
        } else {
            // Use the viewer's block callback so lists, quotes and reference links
            // still belong to one Markdown document.
            let (markdown, marker) = with_placeholders(text, &blocks);
            let table_cache = RefCell::new(table_cache);
            let table_link = table_link.clone();
            let targets = local_files
                .iter()
                .map(|(target, _)| target.to_string())
                .collect::<Vec<_>>();
            let show_block = move |ui: &mut Ui, html: &str| {
                let slot = html
                    .trim()
                    .strip_prefix(&marker)
                    .and_then(|html| html.strip_suffix(" -->"))
                    .and_then(|index| index.parse::<usize>().ok());
                if let Some((index, (_, block))) =
                    slot.and_then(|index| blocks.get(index).map(|block| (index, block)))
                {
                    if ui.cursor().left() > ui.max_rect().left() {
                        ui.end_row();
                        ui.set_row_height(0.);
                    }
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .id_salt(("markdown block", index))
                            .layout(Layout::top_down(Align::Min)),
                        |ui| match block {
                            Block::Code(code) => render_code_block(ui, code),
                            Block::Table(source) => {
                                let mut cache = table_cache.borrow_mut();
                                // A wide grid can expand egui's parent layout. Contain that
                                // width so prose and the chat scrollbar stay in the viewport.
                                egui::ScrollArea::horizontal().min_scrolled_width(0.).show(
                                    ui,
                                    |ui| {
                                        let viewer =
                                            CommonMarkViewer::new().explicit_image_uri_scheme(true);
                                        show_markdown(ui, &mut cache, source, viewer, remote);
                                    },
                                );
                                if let Some(index) = targets
                                    .iter()
                                    .position(|target| cache.get_link_hook(target) == Some(true))
                                {
                                    table_link.set(Some(index));
                                }
                            }
                        },
                    );
                    ui.end_row();
                    // A wrapped row inherits the height of the previous widget.
                    ui.set_row_height(0.);
                } else {
                    ui.label(html);
                }
            };
            let viewer = viewer.render_html_fn(Some(&show_block));
            show_markdown(ui, cache, &markdown, viewer, remote);
        }
    });
    local_files
        .into_iter()
        .enumerate()
        .find(|(index, (target, _))| {
            table_link.get() == Some(*index) || cache.get_link_hook(target) == Some(true)
        })
        .map(|(_, (_, path))| path)
}

fn show_markdown(
    ui: &mut Ui,
    cache: &mut CommonMarkCache,
    source: &str,
    viewer: CommonMarkViewer<'_>,
    remote: bool,
) {
    if !remote {
        viewer.show(ui, cache, source);
        return;
    }
    let text = image_alt_text(source, cache);
    // Rewriting alt text can change how neighboring Markdown is parsed.
    if let Cow::Owned(text) = &text {
        for event in Parser::new_ext(text, parser_options()) {
            match event {
                Event::Start(Tag::Image { .. }) => {
                    ui.label(text);
                    return;
                }
                Event::Start(Tag::Link { dest_url, .. }) if !web_link(&dest_url) => {
                    cache.add_link_hook(dest_url.as_ref());
                }
                _ => {}
            }
        }
    }
    viewer.show(ui, cache, &text);
}

fn image_alt_text<'a>(source: &'a str, cache: &mut CommonMarkCache) -> Cow<'a, str> {
    let mut text = String::new();
    let mut cursor = 0;
    let mut image_depth = 0;
    for (event, range) in Parser::new_ext(source, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) if !web_link(&dest_url) => {
                cache.add_link_hook(dest_url.as_ref());
            }
            Event::Start(Tag::Image { .. }) => {
                if image_depth == 0 {
                    text.push_str(&source[cursor..range.start]);
                    cursor = range.end;
                }
                image_depth += 1;
            }
            Event::End(TagEnd::Image) => image_depth -= 1,
            Event::Text(alt) | Event::Code(alt) if image_depth != 0 => {
                for character in alt.chars() {
                    if character.is_ascii_punctuation() {
                        text.push('\\');
                    }
                    text.push(character);
                }
            }
            Event::SoftBreak | Event::HardBreak if image_depth != 0 => text.push(' '),
            _ => {}
        }
    }
    if cursor == 0 {
        Cow::Borrowed(source)
    } else {
        text.push_str(&source[cursor..]);
        Cow::Owned(text)
    }
}

enum Block {
    Code(CodeBlock),
    Table(String),
}

struct TableBlock<'a> {
    alignments: Vec<Alignment>,
    source: String,
    references: BTreeMap<String, CowStr<'a>>,
}

impl<'a> TableBlock<'a> {
    fn add_header(&mut self, header: &str) {
        // Row ranges start after the list or quote prefix. Rebuild the delimiter
        // line so the table can be rendered independently of those containers.
        self.source.push_str(header);
        self.source.push('|');
        for alignment in &self.alignments {
            self.source.push_str(match alignment {
                Alignment::None => " --- |",
                Alignment::Left => " :--- |",
                Alignment::Center => " :---: |",
                Alignment::Right => " ---: |",
            });
        }
        self.source.push('\n');
    }

    fn add_reference(&mut self, id: CowStr<'a>, dest: &CowStr<'a>) {
        if !id.is_empty() {
            self.references
                .entry(id.into_string())
                .or_insert_with(|| dest.clone());
        }
    }

    fn finish(mut self) -> String {
        self.source.push('\n');
        for (label, dest) in self.references {
            let dest = dest
                .replace('&', "&amp;")
                .replace('\\', "\\\\")
                .replace('<', "\\<")
                .replace('>', "\\>")
                .replace('\r', "&#13;")
                .replace('\n', "&#10;");
            let _ = write!(self.source, "\n[{label}]: <{dest}>");
        }
        self.source
    }
}

struct CodeBlock {
    info: String,
    content: String,
}

impl CodeBlock {
    fn start(
        text: &str,
        mut range: Range<usize>,
        kind: CodeBlockKind<'_>,
    ) -> Option<(Range<usize>, Self)> {
        let info = match kind {
            CodeBlockKind::Fenced(info) => info.into_string(),
            CodeBlockKind::Indented => {
                let prefix = &text[..range.start];
                let spaces = prefix
                    .bytes()
                    .rev()
                    .take_while(|&byte| byte == b' ')
                    .take(4)
                    .count();
                range.start -= spaces;
                if spaces < 4 {
                    if !text[..range.start].ends_with('\t') {
                        return None;
                    }
                    range.start -= 1;
                }
                String::new()
            }
        };
        Some((
            range,
            Self {
                info,
                content: String::new(),
            },
        ))
    }
}

fn with_placeholders(text: &str, blocks: &[(Range<usize>, Block)]) -> (String, String) {
    let mut marker = "<!-- naro-block:".to_owned();
    while text.contains(&marker) {
        marker.push(':');
    }
    let mut markdown = String::new();
    let mut cursor = 0;
    for (index, (range, _)) in blocks.iter().enumerate() {
        markdown.push_str(&text[cursor..range.start]);
        let _ = write!(markdown, "{marker}{index} -->");
        if text[range.clone()].ends_with('\n') {
            markdown.push('\n');
        }
        cursor = range.end;
    }
    markdown.push_str(&text[cursor..]);
    (markdown, marker)
}

fn code_language(info: &str) -> (&str, &str) {
    let language = info.split_whitespace().next().unwrap_or_default();
    let (syntax, name) = match language.to_ascii_lowercase().as_str() {
        "bash" => ("sh", "Bash"),
        "sh" | "shell" => ("sh", "Shell"),
        "zsh" => ("sh", "Zsh"),
        "rs" | "rust" => ("rs", "Rust"),
        "js" | "javascript" => ("js", "JavaScript"),
        "ts" | "typescript" => ("ts", "TypeScript"),
        "py" | "python" => ("py", "Python"),
        "go" | "golang" => ("go", "Go"),
        "json" => ("json", "JSON"),
        "yaml" | "yml" => ("yml", "YAML"),
        "sql" => ("sql", "SQL"),
        "html" => ("html", "HTML"),
        "css" => ("css", "CSS"),
        "" | "text" | "txt" | "plaintext" => ("", "纯文本"),
        _ => (language, language),
    };
    (syntax, name)
}

fn render_code_block(ui: &mut Ui, block: &CodeBlock) {
    let palette = Palette::of(ui);
    let (syntax, name) = code_language(&block.info);
    let name = Language::in_ui(ui).text(name);
    let content = block.content.strip_suffix('\n').unwrap_or(&block.content);
    let icon = if syntax == "sh" {
        Icon::Terminal
    } else {
        Icon::Code
    };
    surface(palette, palette.paper, Margin::ZERO).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.;

        egui::Frame::new()
            .fill(palette.soft)
            .corner_radius(egui::CornerRadius {
                nw: 11,
                ne: 11,
                ..Default::default()
            })
            .inner_margin(Margin::symmetric(14, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        copy_code_button(ui, content);
                        let title = egui::AtomLayout::new((
                            icon.atom(),
                            RichText::new(name).size(13.).color(palette.ink),
                        ))
                        .align2(egui::Align2::LEFT_CENTER)
                        .gap(8.)
                        .min_size(vec2(ui.available_width().max(0.), 28.))
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .show(ui);
                        icon.paint_in(ui, &title, palette.accent);
                    });
                });
            });

        let (_, divider) = ui.allocate_space(vec2(ui.available_width(), 1.));
        ui.painter().hline(
            divider.x_range(),
            divider.center().y,
            Stroke::new(1., palette.line),
        );

        egui::Frame::new()
            .inner_margin(Margin::symmetric(15, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let theme = CodeTheme::from_style(ui.style());
                        let mut job = syntax_highlighting::highlight(
                            ui.ctx(),
                            ui.style(),
                            &theme,
                            content,
                            syntax,
                        );
                        for section in &mut job.sections {
                            section.format.line_height = Some(20.);
                            section.format.underline = Stroke::NONE;
                        }
                        ui.add(Label::new(job).extend().selectable(true));
                    });
            });
    });
}

fn copy_code_button(ui: &mut Ui, content: &str) {
    let palette = Palette::of(ui);
    let language = Language::in_ui(ui);
    let id = ui.make_persistent_id("last copy");
    let now = ui.input(|input| input.time);
    let copied = ui
        .data(|data| data.get_temp::<f64>(id))
        .is_some_and(|time| now - time < 1.6);
    let (icon, text, color) = if copied {
        (Icon::Check, language.text("已复制"), palette.accent)
    } else {
        (Icon::Copy, language.text("复制"), palette.muted)
    };
    let button = icon
        .label(RichText::new(text).size(12.).color(color))
        .frame_when_inactive(false)
        .min_size(vec2(70., 28.))
        .atom_ui(ui);
    icon.paint_in(ui, &button, color);
    if button
        .response
        .on_hover_text(language.text("复制代码"))
        .clicked()
    {
        ui.ctx().copy_text(content.to_owned());
        ui.data_mut(|data| data.insert_temp(id, now));
        ui.ctx().request_repaint_after(Duration::from_millis(1600));
    }
}

fn local_file(target: &str, cwd: &Path) -> Option<PathBuf> {
    if target.is_empty()
        || target.len() > 4096
        || target.starts_with(['#', '?'])
        || target.starts_with("//")
        || target.starts_with("\\\\")
        || target.chars().any(char::is_control)
    {
        return None;
    }
    let target = target
        .rsplit_once(':')
        .filter(|(_, line)| !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(target, |(path, _)| path);
    let path = Path::new(target);
    if path.is_absolute() && !target.starts_with('/') {
        return Some(path.to_owned());
    }
    let url = if path.is_absolute() {
        Url::parse(&format!("file://{target}")).ok()?
    } else if let Ok(url) = Url::parse(target) {
        url
    } else {
        Url::from_directory_path(cwd).ok()?.join(target).ok()?
    };
    if url.scheme() == "file" && url.host_str().is_none() {
        return url.to_file_path().ok();
    }
    None
}

pub(super) fn reveal_file(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).status();
    #[cfg(target_os = "windows")]
    let _ = Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if let Some(directory) = path.parent() {
        let _ = Command::new("xdg-open").arg(directory).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_markdown_does_not_load_image_uris_or_open_local_links() {
        use egui::load::{LoadError, SizeHint, TextureLoadResult, TextureLoader};
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct RecordingLoader(Mutex<Vec<String>>);

        impl TextureLoader for RecordingLoader {
            fn id(&self) -> &str {
                "remote markdown test"
            }

            fn load(
                &self,
                _ctx: &egui::Context,
                uri: &str,
                _options: egui::TextureOptions,
                _size: SizeHint,
            ) -> TextureLoadResult {
                self.0.lock().unwrap().push(uri.to_owned());
                Err(LoadError::NotSupported)
            }

            fn forget(&self, _uri: &str) {}
            fn forget_all(&self) {}
            fn byte_size(&self) -> usize {
                0
            }
        }

        let document = "[Local document](file:///tmp/private.md)\n\n\
            [Web page](https://example.com)\n\n\
            ![Alt text A](file:///tmp/private.png)\n\n\
            | Image |\n| --- |\n| ![Alt text B][reference] |\n\n\
            [reference]: https://example.com/remote.png";
        let ctx = egui::Context::default();
        let loads = Arc::new(RecordingLoader::default());
        ctx.add_texture_loader(loads.clone());
        let mut cache = CommonMarkCache::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_remote(ui, &mut cache, document);
            });
        });
        assert!(loads.0.lock().unwrap().is_empty());
        assert_eq!(cache.get_link_hook("file:///tmp/private.md"), Some(false));
        assert_eq!(cache.get_link_hook("https://example.com"), None);
        for alt in ["Alt text A", "Alt text B"] {
            assert!(output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains(alt))
            }));
        }

        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render(ui, &mut cache, document, Path::new("/tmp"));
            });
        });
        let loads = loads.0.lock().unwrap();
        assert!(loads.iter().any(|uri| uri == "file:///tmp/private.png"));
        assert!(
            loads
                .iter()
                .any(|uri| uri == "https://example.com/remote.png")
        );
    }

    #[test]
    fn code_panels_preserve_nested_markdown_and_reference_links() {
        let document = r#"Before [example][ref].

> ```bash
> echo hello
> ```
> After the quote's code.

- ```rust
  let value = 1;
  ```

      indented code

[ref]: example.txt
"#;
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Light);
        let mut cache = CommonMarkCache::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(740., 580.),
            )),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let cwd = std::env::temp_dir();
                render(ui, &mut cache, document, &cwd);
            });
        });
        let drawn_text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        for expected in [
            "Bash",
            "echo hello",
            "After the quote's code.",
            "Rust",
            "let value = 1;",
            "indented code",
        ] {
            assert!(
                drawn_text.contains(&expected),
                "missing {expected:?} in {drawn_text:?}"
            );
        }
        assert!(
            !drawn_text
                .iter()
                .any(|text| text.contains("<!-- naro-block"))
        );
        assert_eq!(cache.get_link_hook("example.txt"), Some(false));
    }

    #[test]
    fn code_panels_start_on_a_new_row_after_inline_list_text() {
        let document = r#"Plain paragraph:
```python
print(1)
```

1. **Check file**: run in terminal
   ```bash
   cat ~/.gemini/antigravity-cli/settings.json
   ```
   Confirm both keys exist:
   ```json
   "toolPermission": "always-proceed",
   "enableTerminalSandbox": false
   ```

> Quote note
> ```sh
> echo quoted
> ```
"#;
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Light);
        let mut cache = CommonMarkCache::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(740., 800.),
            )),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let cwd = std::env::temp_dir();
                render(ui, &mut cache, document, &cwd);
            });
        });
        let text_rect = |needle: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing text containing {needle:?}"))
        };

        let paragraph_text = text_rect("Plain paragraph:");
        let python_header = text_rect("Python");
        let first_text = text_rect("run in terminal");
        let bash_header = text_rect("Bash");
        let second_text = text_rect("Confirm both keys exist:");
        let json_header = text_rect("JSON");
        let quote_text = text_rect("Quote note");
        let shell_header = text_rect("Shell");
        let header_bar_rect = |header: egui::Rect| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.fill == Palette::LIGHT.soft
                            && rect.rect.contains(header.center()) =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing code header bar containing {header:?}"))
        };
        let python_bar = header_bar_rect(python_header);
        let bash_bar = header_bar_rect(bash_header);
        let json_bar = header_bar_rect(json_header);
        let shell_bar = header_bar_rect(shell_header);

        assert!(
            python_bar.top() >= paragraph_text.bottom(),
            "Python block ({python_bar:?}) should render below {paragraph_text:?}"
        );
        assert!(
            bash_bar.top() >= first_text.bottom(),
            "Bash block ({bash_bar:?}) should render below {first_text:?}"
        );
        assert!(
            second_text.top() >= bash_bar.bottom(),
            "Follow-up text ({second_text:?}) should render below {bash_bar:?}"
        );
        assert!(
            json_bar.top() >= second_text.bottom(),
            "JSON block ({json_bar:?}) should render below {second_text:?}"
        );
        assert!(
            (bash_bar.left() - second_text.left()).abs() < 2.0,
            "Bash block ({bash_bar:?}) should align with list text column ({second_text:?})"
        );
        assert!(
            (json_bar.left() - second_text.left()).abs() < 2.0,
            "JSON block ({json_bar:?}) should align with list text column ({second_text:?})"
        );
        assert!(
            bash_bar.width() > 600.0 && json_bar.width() > 600.0,
            "Code blocks should span the full list width: {bash_bar:?}, {json_bar:?}"
        );
        assert!(
            shell_bar.top() >= quote_text.bottom(),
            "Quoted code block ({shell_bar:?}) should render below {quote_text:?}"
        );
    }

    #[test]
    fn code_panel_copies_the_multiline_source() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Light);
        let mut cache = CommonMarkCache::default();
        let mut render_frame = |events| {
            ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let document = "```bash\necho first\n  echo second\n```";
                        render(ui, &mut cache, document, &std::env::temp_dir());
                    });
                },
            )
        };
        let rendered = render_frame(Vec::new());
        let position = rendered
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "复制" => {
                    Some(text.pos + text.galley.size() / 2.)
                }
                _ => None,
            })
            .expect("copy button");
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let clicked = render_frame(vec![
            egui::Event::PointerMoved(position),
            button(true),
            button(false),
        ]);
        assert!(clicked.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == "echo first\n  echo second")
        }));
        let feedback = render_frame(Vec::new());
        assert!(feedback.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "已复制")
        }));
    }

    #[test]
    fn reply_links_resolve_files_without_opening_unhandled_schemes() {
        #[cfg(windows)]
        let cwd = Path::new("C:/project");
        #[cfg(not(windows))]
        let cwd = Path::new("/project");
        assert_eq!(
            local_file("src/app.rs:42", cwd),
            Some(cwd.join("src/app.rs"))
        );
        assert_eq!(
            local_file("dist/Naro%20mac.zip", cwd),
            Some(cwd.join("dist/Naro mac.zip"))
        );
        assert_eq!(local_file("https://github.com/example/project", cwd), None);
        assert_eq!(local_file("javascript:alert(1)", cwd), None);
        assert_eq!(local_file("file://another-host/project/main.rs", cwd), None);
        assert_eq!(local_file("//another-host/project/main.rs", cwd), None);
    }
}
