//! The window's look: each theme's colours over glass the desktop shows
//! through, the type, and the few touches of depth: spheres lit from above,
//! a line of light along the pane with the focus, text a light runs along.

use bevy_egui::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Mesh, Painter, Pos2, Shadow, Stroke,
    TextFormat, TextStyle, Visuals, pos2, text::LayoutJob, vec2,
};
use iq_core::Agent;
use iq_ui::costs::Payer;
use iq_ui::look::{Look, Theme};

/// The size body text is drawn at, in points.
pub(crate) const BODY_SIZE: f32 = 14.5;

/// The size code and tools are drawn at.
pub(crate) const MONO_SIZE: f32 = 12.8;

/// The family headings and names are drawn in.
const BOLD: &str = "inter-semibold";

/// The family figures and code are drawn in when they stand out.
const MONO_MEDIUM: &str = "mono-medium";

/// A theme's colours, as opaque as the window is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Palette {
    /// Whether it is dark.
    pub(crate) dark: bool,
    /// Between the panes: the glass the desktop shows through most.
    pub(crate) ground: Color32,
    /// A pane.
    pub(crate) pane: Color32,
    /// The pane with the focus, a step closer.
    pub(crate) pane_focus: Color32,
    /// Cards and chips within a pane.
    pub(crate) card: Color32,
    /// The edge of a pane or a card.
    pub(crate) edge: Color32,
    /// The text.
    pub(crate) text: Color32,
    /// What is said in passing.
    pub(crate) dim: Color32,
    /// An agent at rest.
    pub(crate) idle: Color32,
    /// The theme's one colour: the focus, what works, the autopilot.
    pub(crate) accent: Color32,
    /// A second, quieter colour, for what is neither good nor bad.
    pub(crate) info: Color32,
    /// What went well, and what was added.
    pub(crate) ok: Color32,
    /// What failed, and what was removed.
    pub(crate) bad: Color32,
    /// What waits for the person.
    pub(crate) warn: Color32,
    /// Behind what the person wrote.
    pub(crate) you: Color32,
    /// Behind code.
    pub(crate) code: Color32,
    /// Behind what the person is writing.
    pub(crate) input: Color32,
    /// The empty part of a bar.
    pub(crate) track: Color32,
    /// The shadow under a pane.
    pub(crate) shadow: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// `colour` with `alpha`, from 0 to 1, of its own opacity.
pub(crate) fn alpha(colour: Color32, alpha: f32) -> Color32 {
    let [r, g, b, a] = colour.to_srgba_unmultiplied();
    let a = (f32::from(a) * alpha.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// `colour` made opaque.
pub(crate) fn opaque(colour: Color32) -> Color32 {
    let [r, g, b, _] = colour.to_srgba_unmultiplied();
    Color32::from_rgb(r, g, b)
}

/// `a` turned toward `b` by `t`, from 0 to 1.
pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let [ar, ag, ab, aa] = a.to_srgba_unmultiplied();
    let [br, bg, bb, ba] = b.to_srgba_unmultiplied();
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        channel(ar, br),
        channel(ag, bg),
        channel(ab, bb),
        channel(aa, ba),
    )
}

/// `look`'s colours: its theme's, the glass as opaque as it says.
///
/// The dark themes are dark through and through: what stands apart within
/// a pane, code, cards, what the person writes, is set deeper into it, in
/// black, never lifted in grey.
pub(crate) fn palette(look: Look) -> Palette {
    let opacity = look.opacity();
    // The panes stay more opaque than the glass between them, so that
    // text always reads: the desktop shows most around them.
    // A light theme over a dark desktop turns grey where it is clear:
    // its glass stays nearly as opaque as its panes.
    let clear = if look.theme() == Theme::Porcelain {
        0.92
    } else {
        0.55
    };
    let glass = |colour: Color32| alpha(colour, opacity * clear);
    let pane = |colour: Color32| alpha(colour, (opacity + (1.0 - opacity) * 0.2).min(1.0));
    let over = Color32::from_rgba_unmultiplied;
    match look.theme() {
        Theme::Graphite => Palette {
            dark: true,
            ground: glass(rgb(3, 3, 4)),
            pane: pane(rgb(8, 8, 10)),
            pane_focus: pane(rgb(11, 11, 13)),
            card: over(0, 0, 0, 90),
            edge: over(255, 255, 255, 16),
            text: rgb(236, 232, 224),
            dim: rgb(150, 145, 136),
            idle: rgb(84, 81, 76),
            accent: rgb(200, 164, 106),
            info: rgb(142, 166, 188),
            ok: rgb(125, 185, 165),
            bad: rgb(224, 138, 114),
            warn: rgb(217, 178, 106),
            you: Color32::TRANSPARENT,
            code: over(0, 0, 0, 110),
            input: over(0, 0, 0, 120),
            track: over(255, 255, 255, 18),
            shadow: over(0, 0, 0, 150),
        },
        Theme::Ink => Palette {
            dark: true,
            ground: glass(rgb(2, 3, 8)),
            pane: pane(rgb(5, 7, 15)),
            pane_focus: pane(rgb(7, 10, 20)),
            card: over(0, 0, 6, 90),
            edge: over(150, 180, 255, 22),
            text: rgb(228, 233, 243),
            dim: rgb(138, 150, 176),
            idle: rgb(68, 76, 96),
            accent: rgb(138, 180, 255),
            info: rgb(195, 166, 255),
            ok: rgb(127, 209, 185),
            bad: rgb(255, 155, 133),
            warn: rgb(231, 194, 118),
            you: Color32::TRANSPARENT,
            code: over(0, 0, 6, 110),
            input: over(0, 0, 6, 120),
            track: over(150, 180, 255, 22),
            shadow: over(0, 0, 0, 170),
        },
        Theme::Porcelain => Palette {
            dark: false,
            ground: glass(rgb(236, 232, 225)),
            pane: pane(rgb(251, 250, 247)),
            pane_focus: pane(rgb(255, 255, 253)),
            card: over(30, 29, 26, 7),
            edge: over(30, 29, 26, 24),
            text: rgb(30, 29, 26),
            dim: rgb(102, 98, 90),
            idle: rgb(184, 179, 169),
            accent: rgb(158, 84, 48),
            info: rgb(47, 94, 138),
            ok: rgb(47, 125, 107),
            bad: rgb(176, 71, 47),
            warn: rgb(168, 116, 26),
            you: over(158, 84, 48, 18),
            code: over(30, 29, 26, 13),
            input: over(255, 255, 255, 200),
            track: over(30, 29, 26, 22),
            shadow: over(60, 48, 30, 40),
        },
    }
}

/// Each payer's colour.
pub(crate) fn payer(palette: &Palette, payer: Payer) -> Color32 {
    match payer {
        Payer::Provider => palette.ok,
        Payer::Subscription(Agent::ClaudeCode) => palette.accent,
        Payer::Subscription(Agent::Codex) => palette.info,
    }
}

/// Text in the theme's bold face, at `size`.
pub(crate) fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// Figures and code that stand out, at `size`.
pub(crate) fn mono_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MONO_MEDIUM.into()))
}

/// Code and tools.
pub(crate) fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Fonts the system may have, for the characters the window's own lack:
/// arrows, boxes, braille, ✓ and ✗.
const FALLBACK_FONTS: [(&str, &str); 5] = [
    (
        "dejavu-sans",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ),
    (
        "noto-sans",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    ),
    (
        "noto-symbols",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
    ),
    (
        "arial-unicode",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    ),
    ("apple-symbols", "/System/Library/Fonts/Apple Symbols.ttf"),
];

/// The window's own fonts, Inter and JetBrains Mono, shipped with it under
/// the SIL Open Font License, then the system's for what they lack; and
/// the sizes text is drawn at.
pub(crate) fn fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let own: [(&str, &'static [u8]); 4] = [
        ("inter", include_bytes!("../fonts/Inter-Regular.ttf")),
        (BOLD, include_bytes!("../fonts/Inter-SemiBold.ttf")),
        ("mono", include_bytes!("../fonts/JetBrainsMono-Regular.ttf")),
        (
            MONO_MEDIUM,
            include_bytes!("../fonts/JetBrainsMono-Medium.ttf"),
        ),
    ];
    for (name, bytes) in own {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let mut fallbacks = Vec::new();
    for (name, path) in FALLBACK_FONTS {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fallbacks.push(name.to_owned());
        }
    }
    let defaults = |family: &FontFamily| fonts.families.get(family).cloned().unwrap_or_default();
    let proportional = defaults(&FontFamily::Proportional);
    let monospace = defaults(&FontFamily::Monospace);
    let family = |first: &str, then: &str, defaults: &[String]| -> Vec<String> {
        let mut list = vec![first.to_owned(), then.to_owned()];
        list.extend(defaults.iter().cloned());
        list.extend(fallbacks.iter().cloned());
        list
    };
    let families = [
        (
            FontFamily::Proportional,
            family("inter", "mono", &proportional),
        ),
        (FontFamily::Monospace, family("mono", "inter", &monospace)),
        (
            FontFamily::Name(BOLD.into()),
            family(BOLD, "inter", &proportional),
        ),
        (
            FontFamily::Name(MONO_MEDIUM.into()),
            family(MONO_MEDIUM, "mono", &monospace),
        ),
    ];
    for (name, list) in families {
        fonts.families.insert(name, list);
    }
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.window_margin = Margin::same(14);
        style.spacing.button_padding = vec2(10.0, 5.0);
        style.text_styles = [
            (TextStyle::Heading, bold(19.0)),
            (
                TextStyle::Body,
                FontId::new(BODY_SIZE, FontFamily::Proportional),
            ),
            (TextStyle::Monospace, mono(MONO_SIZE)),
            (
                TextStyle::Button,
                FontId::new(BODY_SIZE - 1.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
        ]
        .into();
    });
}

/// The characters the window writes most: what models write in English
/// and French, code, and the window's own marks.
const WRITTEN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 \
    .,;:!?'\"`~@#$%^&*()[]{}<>/\\|-_+=éèêàâîôùûçÉÀ«»’…·●›⎿✉⏎⏵→";

/// Lays out what the window writes most, in the faces and sizes it writes
/// it in, as the window opens: egui draws a character into its atlas the
/// first time it is laid out, and the frame that grows the atlas shows
/// text with characters missing. Only those it uses: an atlas filled past
/// four fifths is cleared and drawn again, which shows the same.
pub(crate) fn warm(ctx: &egui::Context) {
    let proportional = |size| FontId::new(size, FontFamily::Proportional);
    let faces = [
        proportional(BODY_SIZE),
        proportional(BODY_SIZE - 1.0),
        proportional(12.0),
        proportional(18.0),
        mono(MONO_SIZE),
        mono(MONO_SIZE - 0.8),
        mono(MONO_SIZE - 1.0),
        mono(MONO_SIZE + 1.0),
        mono(11.0),
        mono(11.5),
        mono(12.0),
        bold(13.0),
        bold(15.0),
        bold(16.0),
        bold(19.0),
        mono_medium(MONO_SIZE),
        mono_medium(12.0),
        mono_medium(21.0),
    ];
    ctx.fonts_mut(|fonts| {
        for face in faces {
            let _ = fonts.layout_no_wrap(WRITTEN.to_owned(), face, Color32::WHITE);
        }
    });
}

/// Gives egui's own widgets the palette's colours: what the window draws
/// itself reads the palette directly.
pub(crate) fn visuals(ctx: &egui::Context, palette: &Palette) {
    let mut visuals = if palette.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.panel_fill = palette.ground;
    // Menus and tooltips stay opaque: over text, see-through, they could
    // not be read.
    visuals.window_fill = opaque(palette.pane_focus);
    visuals.extreme_bg_color = palette.input;
    visuals.faint_bg_color = palette.card;
    visuals.code_bg_color = palette.code;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.window_stroke = Stroke::new(1.0, palette.edge);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: palette.shadow,
    };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.selection.bg_fill = alpha(palette.accent, 0.22);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);
    // A blinking cursor asks for a frame at each blink, and bevy_egui draws
    // any frame asked at once: the input, which keeps the focus, would have
    // the window drawn without rest.
    visuals.text_cursor.blink = false;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.edge);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    let rest = if palette.dark {
        Color32::TRANSPARENT
    } else {
        palette.card
    };
    for (state, fill) in [
        (&mut visuals.widgets.inactive, rest),
        (&mut visuals.widgets.hovered, alpha(palette.accent, 0.14)),
        (&mut visuals.widgets.active, alpha(palette.accent, 0.24)),
        (&mut visuals.widgets.open, alpha(palette.accent, 0.18)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(7);
        state.fg_stroke = Stroke::new(1.0, palette.text);
    }
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.edge);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, alpha(palette.accent, 0.5));
    ctx.set_visuals(visuals);
}

/// A sphere lit from the top left, at `center`: an agent, glowing as much
/// as `glow` says, from 0 at rest to 1.
pub(crate) fn sphere(painter: &Painter, center: Pos2, radius: f32, colour: Color32, glow: f32) {
    if glow > 0.0 {
        for (scale, share) in [(2.6, 0.10), (1.9, 0.18), (1.45, 0.28)] {
            painter.circle_filled(center, radius * scale, alpha(colour, share * glow));
        }
    }
    let light = pos2(center.x - radius * 0.32, center.y - radius * 0.34);
    let lit = mix(colour, Color32::WHITE, 0.72);
    let shade = mix(colour, Color32::BLACK, 0.5);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(light, lit);
    const STEPS: u32 = 36;
    for step in 0..STEPS {
        let angle = std::f32::consts::TAU * step as f32 / STEPS as f32;
        let edge = center + vec2(angle.cos(), angle.sin()) * radius;
        // Halfway to the edge, the sphere's own colour; at the edge, its
        // shade.
        mesh.colored_vertex(light + (edge - light) * 0.55, colour);
        mesh.colored_vertex(edge, shade);
    }
    for step in 0..STEPS {
        let inner = 1 + 2 * step;
        let outer = inner + 1;
        let next_inner = 1 + 2 * ((step + 1) % STEPS);
        let next_outer = next_inner + 1;
        mesh.add_triangle(0, inner, next_inner);
        mesh.add_triangle(inner, outer, next_outer);
        mesh.add_triangle(inner, next_outer, next_inner);
    }
    painter.add(mesh);
}

/// A line of light from `left` to `right` at height `y`: clear at its
/// ends, `colour` in its middle.
pub(crate) fn light_line(painter: &Painter, left: f32, right: f32, y: f32, colour: Color32) {
    let middle = (left + right) / 2.0;
    let mut mesh = Mesh::default();
    for (x, c) in [
        (left, alpha(colour, 0.0)),
        (middle, colour),
        (right, alpha(colour, 0.0)),
    ] {
        mesh.colored_vertex(pos2(x, y), c);
        mesh.colored_vertex(pos2(x, y + 1.2), c);
    }
    for at in [0, 2] {
        mesh.add_triangle(at, at + 1, at + 2);
        mesh.add_triangle(at + 1, at + 3, at + 2);
    }
    painter.add(mesh);
}

/// `text` with a light running along it, at `phase`, from 0 to 1 across.
pub(crate) fn shimmer(text: &str, phase: f32, base: Color32, light: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let count = text.chars().count().max(1) as f32;
    // The light enters before the first character and leaves after the
    // last, so that it seems to pass rather than jump.
    let at = phase * (count + 8.0) - 4.0;
    for (index, c) in text.chars().enumerate() {
        let distance = (index as f32 - at).abs();
        let shine = (1.0 - distance / 4.0).max(0.0);
        let mut buffer = [0u8; 4];
        job.append(
            c.encode_utf8(&mut buffer),
            0.0,
            TextFormat {
                font_id: mono(MONO_SIZE),
                color: mix(base, light, shine),
                ..TextFormat::default()
            },
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_stay_more_opaque_than_the_glass_and_opaque_means_opaque() {
        for theme in Theme::ALL {
            let clear = palette(Look::new(theme, 0.58));
            assert!(clear.pane.a() > clear.ground.a());
            let solid = palette(Look::new(theme, 1.0));
            assert_eq!((solid.pane.a(), solid.pane_focus.a()), (255, 255));
        }
    }

    #[test]
    fn a_light_runs_along_text_from_one_end_to_the_other() {
        // Characters of one colour may share a section: each is found by
        // where it is.
        let colour_at = |phase: f32, index: usize| {
            let job = shimmer("Thinking", phase, Color32::BLACK, Color32::WHITE);
            job.sections
                .iter()
                .find(|section| section.byte_range.contains(&egui::text::ByteIndex(index)))
                .map_or(Color32::TRANSPARENT, |section| section.format.color)
        };
        assert_eq!(colour_at(0.0, 7), Color32::BLACK);
        assert_ne!(colour_at(0.75, 7), Color32::BLACK);
        assert_eq!(colour_at(0.75, 0), Color32::BLACK);
    }
}
