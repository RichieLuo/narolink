use eframe::egui::{
    self, Atom, Button, Color32, FontFamily, FontId, Id, Margin, Painter, Pos2, Rect, RichText,
    Stroke, StrokeKind, TextStyle, Ui, WidgetInfo, WidgetType, vec2,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub(super) const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub(super) fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.code() == code)
    }

    pub(super) fn code(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        }
    }

    pub(super) fn apply(self, ctx: &egui::Context) {
        let (preference, window) = match self {
            Self::System => (
                egui::ThemePreference::System,
                egui::SystemTheme::SystemDefault,
            ),
            Self::Light => (egui::ThemePreference::Light, egui::SystemTheme::Light),
            Self::Dark => (egui::ThemePreference::Dark, egui::SystemTheme::Dark),
        };
        ctx.set_theme(preference);
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(window));
        ctx.request_repaint();
    }
}

/// Semantic colors shared by every desktop surface, including custom-painted widgets.
#[derive(Clone, Copy, Debug)]
pub(super) struct Palette {
    pub canvas: Color32,
    pub sidebar: Color32,
    pub paper: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub line: Color32,
    pub soft: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub tint: Color32,
    pub tint_line: Color32,
    pub accent_soft: Color32,
    pub danger: Color32,
    pub danger_surface: Color32,
    pub danger_hover: Color32,
    pub warn: Color32,
    pub notice: Color32,
    pub added: Color32,
    pub online: Color32,
    pub online_bg: Color32,
    pub composer_text: Color32,
    pub composer_border: Color32,
    pub placeholder: Color32,
    pub selection: Color32,
    pub send_disabled: Color32,
    pub send_disabled_text: Color32,
    pub send_hover: Color32,
    pub backdrop: Color32,
    pub shadow: Color32,
}

impl Palette {
    pub(super) const LIGHT: Self = Self {
        canvas: Color32::from_rgb(245, 248, 252),
        sidebar: Color32::from_rgb(243, 246, 251),
        paper: Color32::WHITE,
        ink: Color32::from_rgb(17, 22, 29),
        muted: Color32::from_rgb(74, 85, 101),
        line: Color32::from_rgb(220, 229, 240),
        soft: Color32::from_rgb(238, 243, 249),
        accent: Color32::from_rgb(53, 112, 181),
        accent_hover: Color32::from_rgb(41, 94, 160),
        on_accent: Color32::WHITE,
        tint: Color32::from_rgb(230, 240, 251),
        tint_line: Color32::from_rgb(189, 214, 240),
        accent_soft: Color32::from_rgb(216, 232, 250),
        danger: Color32::from_rgb(186, 62, 76),
        danger_surface: Color32::from_rgb(251, 239, 241),
        danger_hover: Color32::from_rgb(245, 223, 227),
        warn: Color32::from_rgb(54, 94, 138),
        notice: Color32::from_rgb(242, 247, 253),
        added: Color32::from_rgb(50, 143, 108),
        online: Color32::from_rgb(58, 143, 122),
        online_bg: Color32::from_rgb(235, 247, 244),
        composer_text: Color32::from_gray(32),
        composer_border: Color32::from_gray(225),
        placeholder: Color32::from_gray(105),
        selection: Color32::from_gray(223),
        send_disabled: Color32::from_gray(241),
        send_disabled_text: Color32::from_gray(112),
        send_hover: Color32::from_gray(58),
        backdrop: Color32::from_rgba_premultiplied(18, 26, 37, 48),
        shadow: Color32::from_rgba_premultiplied(6, 10, 17, 16),
    };

    pub(super) const DARK: Self = Self {
        canvas: Color32::from_rgb(17, 21, 27),
        sidebar: Color32::from_rgb(20, 24, 32),
        paper: Color32::from_rgb(23, 27, 34),
        ink: Color32::from_rgb(230, 237, 245),
        muted: Color32::from_rgb(166, 179, 197),
        line: Color32::from_rgb(51, 63, 80),
        soft: Color32::from_rgb(34, 42, 53),
        accent: Color32::from_rgb(130, 183, 245),
        accent_hover: Color32::from_rgb(163, 203, 250),
        on_accent: Color32::from_rgb(17, 24, 32),
        tint: Color32::from_rgb(34, 55, 82),
        tint_line: Color32::from_rgb(69, 101, 138),
        accent_soft: Color32::from_rgb(39, 63, 92),
        danger: Color32::from_rgb(255, 148, 161),
        danger_surface: Color32::from_rgb(59, 36, 45),
        danger_hover: Color32::from_rgb(76, 43, 53),
        warn: Color32::from_rgb(164, 201, 240),
        notice: Color32::from_rgb(30, 46, 65),
        added: Color32::from_rgb(122, 215, 172),
        online: Color32::from_rgb(123, 216, 189),
        online_bg: Color32::from_rgb(28, 52, 50),
        composer_text: Color32::from_rgb(230, 237, 245),
        composer_border: Color32::from_rgb(51, 63, 80),
        placeholder: Color32::from_rgb(166, 179, 197),
        selection: Color32::from_rgb(51, 76, 107),
        send_disabled: Color32::from_rgb(34, 42, 53),
        send_disabled_text: Color32::from_rgb(166, 179, 197),
        send_hover: Color32::from_rgb(203, 215, 230),
        backdrop: Color32::from_black_alpha(140),
        shadow: Color32::from_black_alpha(80),
    };

    pub(super) fn of(ui: &Ui) -> Self {
        Self::for_theme(if ui.visuals().dark_mode {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        })
    }

    pub(super) fn from_context(ctx: &egui::Context) -> Self {
        Self::for_theme(ctx.theme())
    }

    fn for_theme(theme: egui::Theme) -> Self {
        match theme {
            egui::Theme::Light => Self::LIGHT,
            egui::Theme::Dark => Self::DARK,
        }
    }
}

pub(super) fn setup(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::System);
    ctx.options_mut(|options| options.fallback_theme = egui::Theme::Light);
    let mut fonts = egui::FontDefinitions::default();
    let cjk = "noto-cjk".to_owned();
    fonts.font_data.insert(
        cjk.clone(),
        egui::FontData::from_static(include_bytes!("../../assets/NotoSansCJKsc-Regular.otf"))
            .into(),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, cjk.clone());
    }
    if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
        // The emoji font also defines fullwidth ！ and ？. Resolve them as CJK text.
        let emoji = family
            .iter()
            .position(|name| matches!(name.as_str(), "NotoEmoji-Regular" | "emoji-icon-font"))
            .unwrap_or(family.len());
        family.insert(emoji, cjk);
    }
    ctx.set_fonts(fonts);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let palette = Palette::for_theme(theme);
        let mut visuals = match theme {
            egui::Theme::Light => egui::Visuals::light(),
            egui::Theme::Dark => egui::Visuals::dark(),
        };
        visuals.text_alpha_from_coverage = egui::epaint::AlphaFromCoverage::Gamma(0.78);
        visuals.override_text_color = Some(palette.ink);
        visuals.weak_text_color = Some(palette.muted);
        visuals.panel_fill = palette.canvas;
        visuals.window_fill = palette.paper;
        visuals.window_stroke = Stroke::new(1., palette.line);
        visuals.extreme_bg_color = palette.soft;
        visuals.faint_bg_color = palette.soft;
        visuals.code_bg_color = palette.soft;
        visuals.hyperlink_color = palette.accent;
        visuals.selection.bg_fill = palette.tint;
        visuals.selection.stroke = Stroke::new(1., palette.accent);
        visuals.text_cursor.stroke.color = palette.accent;
        visuals.window_shadow.color = palette.shadow;
        visuals.popup_shadow.color = palette.shadow;
        let widgets = &mut visuals.widgets;
        widgets.noninteractive.fg_stroke.color = palette.ink;
        widgets.noninteractive.bg_stroke.color = palette.line;
        for (widget, fill, border) in [
            (&mut widgets.inactive, palette.paper, palette.line),
            (&mut widgets.hovered, palette.soft, palette.line),
            (&mut widgets.active, palette.tint, palette.tint_line),
            (&mut widgets.open, palette.soft, palette.line),
        ] {
            widget.weak_bg_fill = fill;
            widget.bg_fill = fill;
            widget.bg_stroke = Stroke::new(1., border);
            widget.fg_stroke.color = palette.ink;
            widget.corner_radius = 8.into();
            widget.expansion = 0.;
        }
        ctx.set_visuals_of(theme, visuals);
    }
    ctx.all_styles_mut(|style| {
        style.animation_time = 0.14;
        style.spacing.item_spacing = vec2(8., 7.);
        style.spacing.button_padding = vec2(10., 4.);
        style.spacing.interact_size.y = 28.;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(15.));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(13.));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(22.));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.));
    });
}

pub(super) fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0., 1.);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        mix(from.a(), to.a()),
    )
}

pub(super) fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1. - t.clamp(0., 1.);
    1. - inv * inv * inv
}

pub(super) fn solid(
    ui: &mut Ui,
    fill: Color32,
    hover: Color32,
    draw: impl FnOnce(&mut Ui) -> egui::Response,
) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (widget, color) in [
            (&mut widgets.inactive, fill),
            (&mut widgets.hovered, hover),
            (&mut widgets.active, hover),
        ] {
            widget.weak_bg_fill = color;
            widget.bg_fill = color;
            widget.bg_stroke = Stroke::NONE;
        }
        draw(ui)
    })
    .inner
}

pub(super) fn surface(palette: Palette, fill: Color32, padding: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(padding)
        .stroke(Stroke::new(1., palette.line))
        .corner_radius(12)
}

pub(super) fn popup(palette: Palette, padding: Margin) -> egui::Frame {
    surface(palette, palette.paper, padding).shadow(egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: palette.shadow,
    })
}

#[derive(Clone, Copy, Hash)]
pub(super) enum Icon {
    Plus,
    More,
    Search,
    Folder,
    Devices,
    Usage,
    Refresh,
    Image,
    Send,
    Stop,
    Copy,
    Close,
    Right,
    Down,
    Check,
    Lightning,
    LightningFilled,
    Lock,
    Unlock,
    ShieldCheck,
    Sliders,
    Pencil,
    Document,
    Code,
    Terminal,
    Blocked,
    GitBranch,
}

impl Icon {
    pub(super) fn permission(mode: &str) -> Self {
        match mode {
            "request-approval" | "manual" | "default" => Self::Lock,
            "full-access" | "bypassPermissions" => Self::Unlock,
            "approve-for-me" | "auto" => Self::ShieldCheck,
            "acceptEdits" | "accept-edits" => Self::Pencil,
            "plan" => Self::Document,
            "dontAsk" => Self::Blocked,
            _ => Self::Sliders,
        }
    }

    pub(super) fn atom(self) -> Atom<'static> {
        Atom::custom(Id::new(self), vec2(16., 16.))
    }

    pub(super) fn label(self, label: RichText) -> Button<'static> {
        Button::new((self.atom(), label)).truncate()
    }

    pub(super) fn paint_in(self, ui: &Ui, button: &egui::AtomLayoutResponse, color: Color32) {
        if let Some((_, rect)) = button.custom_rects().find(|(id, _)| *id == Id::new(self)) {
            self.paint(ui.painter(), rect.center(), color);
        }
    }

    pub(super) fn button(self, ui: &mut Ui, name: &str, color: Color32) -> egui::Response {
        let button = self.button_layout(ui, name);
        self.paint_in(ui, &button, color);
        button.response
    }

    pub(super) fn button_layout(self, ui: &mut Ui, name: &str) -> egui::AtomLayoutResponse {
        let mut button = Button::new(self.atom())
            .min_size(vec2(30., 29.))
            .frame_when_inactive(false)
            .atom_ui(ui);
        button.response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, button.response.enabled(), name)
        });
        button.response = button.response.on_hover_text(name);
        button
    }

    pub(super) fn refresh_button(
        ui: &mut Ui,
        name: &str,
        color: Color32,
        spinning: bool,
    ) -> egui::Response {
        let button = Button::new(Self::Refresh.atom())
            .min_size(vec2(30., 29.))
            .frame_when_inactive(false)
            .atom_ui(ui);
        if let Some((_, rect)) = button
            .custom_rects()
            .find(|(id, _)| *id == Id::new(Self::Refresh))
        {
            if spinning {
                let angle = (ui.input(|input| input.time) * std::f64::consts::TAU * 1.15) as f32;
                Self::paint_refresh_rotated(
                    ui.painter(),
                    rect.center(),
                    angle,
                    Palette::of(ui).accent,
                );
                ui.ctx().request_repaint();
            } else {
                Self::Refresh.paint(ui.painter(), rect.center(), color);
            }
        }
        let response = button.response;
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, response.enabled(), name));
        response.on_hover_text(name)
    }

    pub(super) fn paint_chevron_in(
        ui: &Ui,
        button: &egui::AtomLayoutResponse,
        openness: f32,
        color: Color32,
    ) {
        if let Some((_, rect)) = button
            .custom_rects()
            .find(|(id, _)| *id == Id::new(Self::Right))
        {
            Self::paint_chevron(ui.painter(), rect.center(), openness, color);
        }
    }

    pub(super) fn paint_chevron(painter: &Painter, c: Pos2, openness: f32, color: Color32) {
        let angle = openness.clamp(0., 1.) * std::f32::consts::FRAC_PI_2;
        let (sin, cos) = angle.sin_cos();
        let rotate = |(x, y): (f32, f32)| c + vec2(x * cos - y * sin, x * sin + y * cos);
        let stroke = Stroke::new(1.45, color);
        painter.add(egui::Shape::line(
            [(-1.5, -3.5), (2., 0.), (-1.5, 3.5)]
                .into_iter()
                .map(rotate)
                .collect(),
            stroke,
        ));
    }

    fn paint_refresh_rotated(painter: &Painter, c: Pos2, angle: f32, color: Color32) {
        let (sin, cos) = angle.sin_cos();
        let rotate = |(x, y): (f32, f32)| c + vec2(x * cos - y * sin, x * sin + y * cos);
        let stroke = Stroke::new(1.45, color);
        let arc: &[(f32, f32)] = &[
            (-4.8, -1.6),
            (-3.4, -4.),
            (-0.5, -5.1),
            (2.6, -4.4),
            (4.7, -2.),
            (5., 0.8),
            (3.6, 3.4),
            (0.7, 4.7),
            (-2.3, 4.1),
            (-4., 2.7),
        ];
        painter.add(egui::Shape::line(
            arc.iter().copied().map(rotate).collect(),
            stroke,
        ));
        painter.add(egui::Shape::line(
            [(-5.1, -4.6), (-5.1, -1.4), (-1.9, -1.4)]
                .into_iter()
                .map(rotate)
                .collect(),
            stroke,
        ));
    }

    pub(super) fn paint(self, painter: &Painter, c: Pos2, color: Color32) {
        let stroke = Stroke::new(1.45, color);
        let point = |x, y| c + vec2(x, y);
        let line = |a: (f32, f32), b: (f32, f32)| {
            painter.line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
        };
        let path = |points: &[(f32, f32)]| {
            painter.add(egui::Shape::line(
                points.iter().map(|&(x, y)| point(x, y)).collect(),
                stroke,
            ));
        };
        let rect = |x1, y1, x2, y2, radius| {
            painter.rect_stroke(
                Rect::from_min_max(point(x1, y1), point(x2, y2)),
                radius,
                stroke,
                StrokeKind::Middle,
            );
        };
        match self {
            Self::Plus => {
                // Align both strokes to the same pixel grid so each arm is the same length.
                let scale = painter.pixels_per_point();
                let stroke_pixels = (stroke.width * scale).round().max(1.);
                let arm_pixels = (5. * scale).round().max(1.);
                let half_stroke = vec2(stroke_pixels, stroke_pixels) * 0.5;
                let center = ((c.to_vec2() * scale - half_stroke).round() + half_stroke) / scale;
                let width = stroke_pixels / scale;
                let length = (stroke_pixels + 2. * arm_pixels) / scale;
                let mut plus = egui::Mesh::default();
                plus.add_colored_rect(
                    Rect::from_center_size(center.to_pos2(), vec2(length, width)),
                    color,
                );
                plus.add_colored_rect(
                    Rect::from_center_size(center.to_pos2(), vec2(width, length)),
                    color,
                );
                painter.add(plus);
            }
            Self::More => {
                for x in [-4.5, 0., 4.5] {
                    painter.circle_filled(point(x, 0.), 1.1, color);
                }
            }
            Self::Search => {
                painter.circle_stroke(point(-1., -1.), 4.25, stroke);
                line((2.2, 2.2), (5.9, 5.9));
            }
            Self::Folder => path(&[
                (-6., 5.),
                (-6., -4.),
                (-2., -4.),
                (0., -2.),
                (6., -2.),
                (6., 5.),
                (-6., 5.),
            ]),
            Self::Devices => {
                rect(-7., -5., 2., 2., 1.3);
                line((-5., 5.), (0., 5.));
                line((-2.5, 2.), (-2.5, 5.));
                rect(4.2, -1., 8., 6., 1.);
            }
            Self::Usage => {
                line((-4.75, 5.), (-4.75, 0.75));
                line((0., 5.), (0., -4.75));
                line((4.75, 5.), (4.75, -1.75));
            }
            Self::Refresh => {
                path(&[
                    (-4.8, -1.6),
                    (-3.4, -4.),
                    (-0.5, -5.1),
                    (2.6, -4.4),
                    (4.7, -2.),
                    (5., 0.8),
                    (3.6, 3.4),
                    (0.7, 4.7),
                    (-2.3, 4.1),
                    (-4., 2.7),
                ]);
                path(&[(-5.1, -4.6), (-5.1, -1.4), (-1.9, -1.4)]);
            }
            Self::Image => {
                rect(-6., -5., 6., 5., 1.5);
                painter.circle_filled(point(-2.6, -1.7), 1.1, color);
                path(&[(-5.1, 3.7), (-1., 0.8), (1.1, 2.2), (3.4, -0.1), (5.4, 1.9)]);
            }
            Self::Send => {
                path(&[(-4.5, -0.4), (0., -4.8), (4.5, -0.4)]);
                line((0., -4.8), (0., 5.));
            }
            Self::Stop => {
                painter.rect_filled(Rect::from_center_size(c, vec2(8., 8.)), 1.4, color);
            }
            Self::Copy => {
                rect(-2.5, -2.8, 5.8, 5.8, 1.2);
                path(&[(-0.5, -5.5), (-5.1, -5.5), (-5.1, 3.)]);
            }
            Self::Close => {
                line((-4., -4.), (4., 4.));
                line((4., -4.), (-4., 4.));
            }
            Self::Right => path(&[(-1.5, -3.5), (2., 0.), (-1.5, 3.5)]),
            Self::Down => path(&[(-3.5, -1.5), (0., 2.), (3.5, -1.5)]),
            Self::Check => path(&[(-4., 0.), (-1.1, 2.8), (4.7, -3.4)]),
            Self::Lightning | Self::LightningFilled => {
                let points = vec![
                    point(1.5, -6.5),
                    point(-4.1, 1.),
                    point(-0.5, 1.),
                    point(-1.5, 6.5),
                    point(4.6, -1.5),
                    point(1., -1.5),
                ];
                if matches!(self, Self::LightningFilled) {
                    let mut mesh = egui::Mesh::default();
                    for &vertex in &points {
                        mesh.colored_vertex(vertex, color);
                    }
                    mesh.indices.extend([2, 3, 4, 2, 4, 5, 2, 5, 0, 2, 0, 1]);
                    painter.add(mesh);
                }
                painter.add(egui::Shape::closed_line(points, stroke));
            }
            Self::Lock | Self::Unlock => {
                rect(-5.25, -0.5, 5.25, 5.5, 1.4);
                path(&[
                    (-3.1, -3.15),
                    (-2.7, -4.4),
                    (-1.5, -5.35),
                    (0., -5.7),
                    (1.5, -5.35),
                    (2.7, -4.4),
                    (3.1, -3.15),
                    (3.1, -0.5),
                ]);
                if matches!(self, Self::Lock) {
                    line((-3.1, -3.15), (-3.1, -0.5));
                }
            }
            Self::ShieldCheck => {
                path(&[
                    (0., -6.2),
                    (5.2, -4.8),
                    (5.2, 0.),
                    (3.4, 4.),
                    (0., 6.2),
                    (-3.4, 4.),
                    (-5.2, 0.),
                    (-5.2, -4.8),
                    (0., -6.2),
                ]);
                path(&[(-2.4, 0.), (-0.5, 1.9), (2.6, -1.5)]);
            }
            Self::Sliders => {
                line((-6., -3.5), (6., -3.5));
                line((-2.5, -5.5), (-2.5, -1.5));
                line((-6., 3.5), (6., 3.5));
                line((2.5, 1.5), (2.5, 5.5));
            }
            Self::Pencil => path(&[
                (-5.5, 5.5),
                (-4.9, 1.9),
                (2.9, -5.9),
                (5.9, -2.9),
                (-1.9, 4.9),
                (-5.5, 5.5),
            ]),
            Self::Document => {
                rect(-4.5, -6., 4.5, 6., 1.4);
                line((-1.7, -2.), (1.7, -2.));
                line((-1.7, 2.), (1.7, 2.));
            }
            Self::Code => {
                path(&[(-4., -3.), (-7., 0.), (-4., 3.)]);
                line((1.4, -4.5), (-1.4, 4.5));
                path(&[(4., -3.), (7., 0.), (4., 3.)]);
            }
            Self::Terminal => {
                rect(-6., -5., 6., 5., 1.5);
                path(&[(-3.7, -1.7), (-1.6, 0.), (-3.7, 1.7)]);
                line((0.1, 1.7), (3.5, 1.7));
            }
            Self::Blocked => {
                painter.circle_stroke(c, 5.4, stroke);
                line((-3.8, 3.8), (3.8, -3.8));
            }
            Self::GitBranch => {
                painter.circle_stroke(point(-3.5, -4.), 1.8, stroke);
                painter.circle_stroke(point(-3.5, 4.), 1.8, stroke);
                painter.circle_stroke(point(3.5, -2.5), 1.8, stroke);
                line((-3.5, -2.2), (-3.5, 2.2));
                path(&[(3.5, -0.7), (3.5, 1.2), (1.8, 2.5), (-1.8, 2.5)]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_codes_round_trip_and_default_to_system() {
        assert_eq!(Appearance::default(), Appearance::System);
        for choice in Appearance::ALL {
            assert_eq!(Appearance::from_code(choice.code()), Some(choice));
        }
        assert_eq!(Appearance::from_code("unknown"), None);
    }

    #[test]
    fn system_changes_update_the_palette_while_explicit_choices_stay_fixed() {
        let ctx = egui::Context::default();
        setup(&ctx);
        for (appearance, system, expected) in [
            (Appearance::System, egui::Theme::Light, egui::Theme::Light),
            (Appearance::System, egui::Theme::Dark, egui::Theme::Dark),
            (Appearance::Light, egui::Theme::Dark, egui::Theme::Light),
            (Appearance::Dark, egui::Theme::Light, egui::Theme::Dark),
            (Appearance::System, egui::Theme::Light, egui::Theme::Light),
        ] {
            appearance.apply(&ctx);
            let _ = ctx.run(
                egui::RawInput {
                    system_theme: Some(system),
                    ..Default::default()
                },
                |ctx| {
                    assert_eq!(ctx.theme(), expected);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let palette = Palette::of(ui);
                        assert_eq!(ui.visuals().dark_mode, expected == egui::Theme::Dark);
                        assert_eq!(ui.visuals().text_color(), palette.ink);
                        assert_eq!(ui.visuals().panel_fill, palette.canvas);
                        assert_eq!(ui.visuals().hyperlink_color, palette.accent);
                        assert_eq!(
                            surface(palette, palette.paper, Margin::ZERO).stroke.color,
                            palette.line
                        );
                    });
                },
            );
        }
    }

    #[test]
    fn dark_text_and_actions_have_readable_contrast() {
        fn luminance(color: Color32) -> f32 {
            let linear = |channel| {
                let value = f32::from(channel) / 255.;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            linear(color.r()) * 0.2126 + linear(color.g()) * 0.7152 + linear(color.b()) * 0.0722
        }
        let palette = Palette::DARK;
        for (foreground, background) in [
            (palette.ink, palette.paper),
            (palette.muted, palette.soft),
            (palette.accent, palette.paper),
            (palette.on_accent, palette.accent),
            (palette.danger, palette.danger_surface),
            (palette.added, palette.canvas),
        ] {
            let a = luminance(foreground);
            let b = luminance(background);
            assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
        }
    }
}
