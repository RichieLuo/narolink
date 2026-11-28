//! Desktop syntax color palette and memoized egui `LayoutJob` builders.
use super::style::Palette;
use crate::syntax::{self, MAX_HIGHLIGHT_BYTES, TokenKind};
use eframe::egui::{
    self, Color32, FontId, Stroke,
    text::{LayoutJob, LayoutSection, TextFormat},
};
use std::path::Path;

#[derive(Clone, Copy)]
struct SyntaxColors {
    plain: Color32,
    comment: Color32,
    keyword: Color32,
    type_name: Color32,
    function: Color32,
    string: Color32,
    number: Color32,
    constant: Color32,
    property: Color32,
    tag: Color32,
    operator: Color32,
    escape: Color32,
    added: Color32,
    removed: Color32,
    diff_added_bg: Color32,
    diff_removed_bg: Color32,
}

impl SyntaxColors {
    const LIGHT: Self = Self {
        plain: Color32::from_rgb(31, 35, 40),
        comment: Color32::from_rgb(110, 119, 129),
        keyword: Color32::from_rgb(207, 34, 46),
        type_name: Color32::from_rgb(149, 56, 0),
        function: Color32::from_rgb(130, 80, 223),
        string: Color32::from_rgb(17, 99, 41),
        number: Color32::from_rgb(5, 80, 174),
        constant: Color32::from_rgb(5, 80, 174),
        property: Color32::from_rgb(14, 116, 144),
        tag: Color32::from_rgb(207, 34, 46),
        operator: Color32::from_rgb(87, 96, 106),
        escape: Color32::from_rgb(149, 56, 0),
        added: Color32::from_rgb(17, 99, 41),
        removed: Color32::from_rgb(186, 62, 76),
        diff_added_bg: Color32::from_rgb(233, 246, 237),
        diff_removed_bg: Color32::from_rgb(252, 236, 238),
    };

    const DARK: Self = Self {
        plain: Color32::from_rgb(230, 237, 245),
        comment: Color32::from_rgb(139, 148, 158),
        keyword: Color32::from_rgb(255, 123, 114),
        type_name: Color32::from_rgb(255, 166, 87),
        function: Color32::from_rgb(210, 168, 255),
        string: Color32::from_rgb(126, 231, 135),
        number: Color32::from_rgb(121, 192, 255),
        constant: Color32::from_rgb(121, 192, 255),
        property: Color32::from_rgb(125, 211, 252),
        tag: Color32::from_rgb(255, 123, 114),
        operator: Color32::from_rgb(166, 179, 197),
        escape: Color32::from_rgb(255, 166, 87),
        added: Color32::from_rgb(126, 231, 135),
        removed: Color32::from_rgb(255, 148, 161),
        diff_added_bg: Color32::from_rgb(28, 52, 40),
        diff_removed_bg: Color32::from_rgb(62, 37, 46),
    };

    fn of(dark_mode: bool) -> Self {
        if dark_mode { Self::DARK } else { Self::LIGHT }
    }

    fn format(
        self,
        kind: TokenKind,
        font_id: FontId,
        line_height: f32,
        background: Color32,
    ) -> TextFormat {
        let color = match kind {
            TokenKind::Plain => self.plain,
            TokenKind::Comment => self.comment,
            TokenKind::Keyword => self.keyword,
            TokenKind::Type => self.type_name,
            TokenKind::Function => self.function,
            TokenKind::String => self.string,
            TokenKind::Number => self.number,
            TokenKind::Constant => self.constant,
            TokenKind::Property => self.property,
            TokenKind::Tag => self.tag,
            TokenKind::Operator => self.operator,
            TokenKind::Escape => self.escape,
            TokenKind::Added => self.added,
            TokenKind::Removed => self.removed,
        };
        TextFormat {
            font_id,
            color,
            background,
            italics: kind == TokenKind::Comment,
            underline: Stroke::NONE,
            line_height: Some(line_height),
            ..Default::default()
        }
    }
}

#[derive(Default)]
struct CodeHighlightComputer;

impl egui::cache::ComputerMut<(u32, u32, bool, &str, &str), LayoutJob> for CodeHighlightComputer {
    fn compute(
        &mut self,
        (font_bits, line_bits, dark_mode, lang, code): (u32, u32, bool, &str, &str),
    ) -> LayoutJob {
        build_code_job(
            code,
            lang,
            f32::from_bits(font_bits),
            f32::from_bits(line_bits),
            dark_mode,
        )
    }
}

type CodeHighlightCache = egui::cache::FrameCache<LayoutJob, CodeHighlightComputer>;

pub(super) fn highlight_job(
    ctx: &egui::Context,
    code: &str,
    lang: &str,
    font_size: f32,
    line_height: f32,
    dark_mode: bool,
) -> LayoutJob {
    if code.is_empty() || code.len() > MAX_HIGHLIGHT_BYTES {
        let colors = SyntaxColors::of(dark_mode);
        let mut job = LayoutJob::simple(
            code.to_owned(),
            FontId::monospace(font_size),
            colors.plain,
            f32::INFINITY,
        );
        job.first_row_min_height = line_height;
        for section in &mut job.sections {
            section.format.line_height = Some(line_height);
        }
        return job;
    }
    ctx.memory_mut(|memory| {
        memory.caches.cache::<CodeHighlightCache>().get((
            font_size.to_bits(),
            line_height.to_bits(),
            dark_mode,
            lang,
            code,
        ))
    })
}

fn build_code_job(
    code: &str,
    lang: &str,
    font_size: f32,
    line_height: f32,
    dark_mode: bool,
) -> LayoutJob {
    let colors = SyntaxColors::of(dark_mode);
    let font = FontId::monospace(font_size);
    let tokens = syntax::tokenize(code, lang);
    let mut job = LayoutJob {
        text: code.to_owned(),
        first_row_min_height: line_height,
        ..Default::default()
    };
    job.sections.reserve(tokens.len());
    for token in tokens {
        job.sections.push(LayoutSection {
            leading_space: 0.,
            byte_range: token.range,
            format: colors.format(token.kind, font.clone(), line_height, Color32::TRANSPARENT),
        });
    }
    job
}

#[derive(Default)]
struct DiffHighlightComputer;

impl egui::cache::ComputerMut<(u32, u32, bool, &str, &str), LayoutJob> for DiffHighlightComputer {
    fn compute(
        &mut self,
        (font_bits, line_bits, dark_mode, default_path, diff): (u32, u32, bool, &str, &str),
    ) -> LayoutJob {
        build_diff_job(
            diff,
            default_path,
            f32::from_bits(font_bits),
            f32::from_bits(line_bits),
            dark_mode,
        )
    }
}

type DiffHighlightCache = egui::cache::FrameCache<LayoutJob, DiffHighlightComputer>;

pub(super) fn highlight_diff_job(
    ctx: &egui::Context,
    diff: &str,
    default_path: &str,
    font_size: f32,
    line_height: f32,
    dark_mode: bool,
) -> LayoutJob {
    if diff.is_empty() || diff.len() > MAX_HIGHLIGHT_BYTES {
        return build_plain_diff_job(diff, font_size, line_height, dark_mode);
    }
    ctx.memory_mut(|memory| {
        memory.caches.cache::<DiffHighlightCache>().get((
            font_size.to_bits(),
            line_height.to_bits(),
            dark_mode,
            default_path,
            diff,
        ))
    })
}

fn build_plain_diff_job(
    diff: &str,
    font_size: f32,
    line_height: f32,
    dark_mode: bool,
) -> LayoutJob {
    let palette = if dark_mode {
        Palette::DARK
    } else {
        Palette::LIGHT
    };
    let font = FontId::monospace(font_size);
    let mut job = LayoutJob {
        first_row_min_height: line_height,
        ..Default::default()
    };
    for line in diff.split_inclusive('\n') {
        let color = if ["+++ ", "--- ", "@@", "diff ", "index ", "*** "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            palette.muted
        } else if line.starts_with('+') {
            palette.added
        } else if line.starts_with('-') {
            palette.danger
        } else {
            palette.ink
        };
        let mut format = TextFormat::simple(font.clone(), color);
        format.line_height = Some(line_height);
        job.append(line, 0., format);
    }
    job
}

/// Build a single-line syntax-highlighted `LayoutJob` with optional emphasized
/// byte ranges (used for inline word/character diff highlights).
pub(super) fn highlight_line_job(
    code: &str,
    lang: &str,
    font_size: f32,
    line_height: f32,
    dark_mode: bool,
    base_bg: Color32,
    emph_bg: Color32,
    emph_ranges: &[std::ops::Range<usize>],
) -> LayoutJob {
    let colors = SyntaxColors::of(dark_mode);
    let font = FontId::monospace(font_size);
    let mut job = LayoutJob {
        first_row_min_height: line_height,
        ..Default::default()
    };
    if code.is_empty() {
        return job;
    }

    let use_syntax = !lang.is_empty() && syntax::resolve_language(lang).is_some();
    let tokens = if use_syntax {
        syntax::tokenize(code, lang)
    } else {
        vec![syntax::Token {
            range: 0..code.len(),
            kind: TokenKind::Plain,
        }]
    };

    if emph_ranges.is_empty() {
        for token in tokens {
            job.append(
                &code[token.range],
                0.,
                colors.format(token.kind, font.clone(), line_height, base_bg),
            );
        }
        return job;
    }

    // Split each syntax token against sorted non-overlapping emphasis ranges.
    let mut emph_idx = 0usize;
    for token in tokens {
        let mut cursor = token.range.start;
        let tok_end = token.range.end;
        while emph_idx < emph_ranges.len() && emph_ranges[emph_idx].end <= cursor {
            emph_idx += 1;
        }
        let mut scan = emph_idx;
        while cursor < tok_end {
            if scan < emph_ranges.len() {
                let er = &emph_ranges[scan];
                if er.start >= tok_end {
                    job.append(
                        &code[cursor..tok_end],
                        0.,
                        colors.format(token.kind, font.clone(), line_height, base_bg),
                    );
                    cursor = tok_end;
                } else if cursor < er.start {
                    let mid = er.start.min(tok_end);
                    job.append(
                        &code[cursor..mid],
                        0.,
                        colors.format(token.kind, font.clone(), line_height, base_bg),
                    );
                    cursor = mid;
                } else {
                    let mid = er.end.min(tok_end);
                    job.append(
                        &code[cursor..mid],
                        0.,
                        colors.format(token.kind, font.clone(), line_height, emph_bg),
                    );
                    cursor = mid;
                    if cursor >= er.end {
                        scan += 1;
                    }
                }
            } else {
                job.append(
                    &code[cursor..tok_end],
                    0.,
                    colors.format(token.kind, font.clone(), line_height, base_bg),
                );
                cursor = tok_end;
            }
        }
    }
    job
}

fn build_diff_job(
    diff: &str,
    default_path: &str,
    font_size: f32,
    line_height: f32,
    dark_mode: bool,
) -> LayoutJob {
    let palette = if dark_mode {
        Palette::DARK
    } else {
        Palette::LIGHT
    };
    let colors = SyntaxColors::of(dark_mode);
    let font = FontId::monospace(font_size);
    let mut current_lang = syntax::detect_syntax_from_path(Path::new(default_path));
    let mut job = LayoutJob {
        first_row_min_height: line_height,
        ..Default::default()
    };

    for line in diff.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let candidate = rest.trim().strip_prefix("b/").unwrap_or(rest.trim());
            if candidate != "/dev/null" {
                let detected = syntax::detect_syntax_from_path(Path::new(candidate));
                if !detected.is_empty() {
                    current_lang = detected;
                }
            }
            let mut format = TextFormat::simple(font.clone(), palette.muted);
            format.line_height = Some(line_height);
            job.append(line, 0., format);
            continue;
        }
        if line.starts_with("@@") {
            let mut format = TextFormat::simple(font.clone(), palette.accent);
            format.line_height = Some(line_height);
            job.append(line, 0., format);
            continue;
        }
        if ["--- ", "diff ", "index ", "*** "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            let mut format = TextFormat::simple(font.clone(), palette.muted);
            format.line_height = Some(line_height);
            job.append(line, 0., format);
            continue;
        }

        let (prefix, code_part, prefix_color, bg) = if let Some(rest) = line.strip_prefix('+') {
            ("+", rest, palette.added, colors.diff_added_bg)
        } else if let Some(rest) = line.strip_prefix('-') {
            ("-", rest, palette.danger, colors.diff_removed_bg)
        } else if let Some(rest) = line.strip_prefix(' ') {
            (" ", rest, palette.muted, Color32::TRANSPARENT)
        } else {
            ("", line, palette.ink, Color32::TRANSPARENT)
        };

        if !prefix.is_empty() {
            let mut prefix_format = TextFormat::simple(font.clone(), prefix_color);
            prefix_format.background = bg;
            prefix_format.line_height = Some(line_height);
            job.append(prefix, 0., prefix_format);
        }

        if current_lang.is_empty() || syntax::resolve_language(&current_lang).is_none() {
            let fallback = match prefix {
                "+" => colors.added,
                "-" => colors.removed,
                _ => colors.plain,
            };
            let mut format = TextFormat::simple(font.clone(), fallback);
            format.background = bg;
            format.line_height = Some(line_height);
            job.append(code_part, 0., format);
        } else {
            let tokens = syntax::tokenize(code_part, &current_lang);
            for token in tokens {
                job.append(
                    &code_part[token.range],
                    0.,
                    colors.format(token.kind, font.clone(), line_height, bg),
                );
            }
        }
    }

    job
}
