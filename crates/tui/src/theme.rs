//! Each theme's colours, over the terminal's own background, which iq
//! never paints: its transparency and its colour stay the person's. In 24
//! bits where the terminal says it takes them, else the nearest of the 256
//! colours every terminal has.

use iq_ui::look::Theme;
use ratatui::style::{Color, Modifier, Style};

/// A colour as iq mixes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rgb(pub(crate) u8, pub(crate) u8, pub(crate) u8);

impl Rgb {
    /// `self` moved toward `other` by `share`, from 0 to 1.
    pub(crate) fn mix(self, other: Rgb, share: f32) -> Rgb {
        let share = share.clamp(0.0, 1.0);
        let along =
            |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * share).round() as u8;
        Rgb(
            along(self.0, other.0),
            along(self.1, other.1),
            along(self.2, other.2),
        )
    }
}

/// How many colours the terminal takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Depth {
    /// Any, in 24 bits.
    True,
    /// The 256 of xterm's palette.
    Indexed,
}

impl Depth {
    /// As the terminal says through `COLORTERM`, the variable terminals
    /// with 24-bit colour set; those that do not set it are given the 256.
    pub(crate) fn detect() -> Depth {
        let said = std::env::var("COLORTERM").unwrap_or_default();
        if said.contains("truecolor") || said.contains("24bit") {
            Depth::True
        } else {
            Depth::Indexed
        }
    }
}

/// The colours of a theme.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Palette {
    depth: Depth,
    /// Whether the theme is for a dark terminal.
    pub(crate) dark: bool,
    /// About the terminal's background, which fades start from.
    pub(crate) ground: Rgb,
    /// The text.
    pub(crate) text: Rgb,
    /// What is said in passing.
    pub(crate) dim: Rgb,
    /// Fainter still: rules, empty tracks, an agent at rest.
    pub(crate) faint: Rgb,
    /// The theme's one colour: what works, what the person writes to.
    pub(crate) accent: Rgb,
    /// A quieter second colour, for what is neither good nor bad.
    pub(crate) info: Rgb,
    /// What went well.
    pub(crate) ok: Rgb,
    /// What failed.
    pub(crate) bad: Rgb,
    /// What waits for the person.
    pub(crate) warn: Rgb,
    /// Code.
    pub(crate) code: Rgb,
    /// Each agent's colour, by the order they joined.
    agents: [Rgb; 6],
}

impl Palette {
    /// The colours of `theme`, at `depth`.
    pub(crate) fn new(theme: Theme, depth: Depth) -> Palette {
        match theme {
            Theme::Graphite => Palette {
                depth,
                dark: true,
                ground: Rgb(12, 12, 14),
                text: Rgb(236, 232, 224),
                dim: Rgb(150, 145, 136),
                faint: Rgb(84, 81, 76),
                accent: Rgb(200, 164, 106),
                info: Rgb(142, 166, 188),
                ok: Rgb(125, 185, 165),
                bad: Rgb(224, 138, 114),
                warn: Rgb(217, 178, 106),
                code: Rgb(181, 196, 210),
                agents: [
                    Rgb(200, 164, 106),
                    Rgb(142, 166, 188),
                    Rgb(125, 185, 165),
                    Rgb(176, 150, 196),
                    Rgb(205, 150, 170),
                    Rgb(214, 196, 150),
                ],
            },
            Theme::Ink => Palette {
                depth,
                dark: true,
                ground: Rgb(8, 10, 18),
                text: Rgb(228, 233, 243),
                dim: Rgb(138, 150, 176),
                faint: Rgb(68, 76, 96),
                accent: Rgb(138, 180, 255),
                info: Rgb(195, 166, 255),
                ok: Rgb(127, 209, 185),
                bad: Rgb(255, 155, 133),
                warn: Rgb(231, 194, 118),
                code: Rgb(170, 205, 235),
                agents: [
                    Rgb(138, 180, 255),
                    Rgb(195, 166, 255),
                    Rgb(127, 209, 185),
                    Rgb(240, 190, 140),
                    Rgb(150, 220, 240),
                    Rgb(230, 160, 210),
                ],
            },
            Theme::Porcelain => Palette {
                depth,
                dark: false,
                ground: Rgb(250, 248, 244),
                text: Rgb(30, 29, 26),
                dim: Rgb(102, 98, 90),
                faint: Rgb(184, 179, 169),
                accent: Rgb(158, 84, 48),
                info: Rgb(47, 94, 138),
                ok: Rgb(47, 125, 107),
                bad: Rgb(176, 71, 47),
                warn: Rgb(168, 116, 26),
                code: Rgb(60, 80, 110),
                agents: [
                    Rgb(158, 84, 48),
                    Rgb(47, 94, 138),
                    Rgb(47, 125, 107),
                    Rgb(110, 70, 140),
                    Rgb(140, 110, 30),
                    Rgb(150, 60, 90),
                ],
            },
        }
    }

    /// `rgb` as the terminal takes it.
    pub(crate) fn color(&self, rgb: Rgb) -> Color {
        match self.depth {
            Depth::True => Color::Rgb(rgb.0, rgb.1, rgb.2),
            Depth::Indexed => Color::Indexed(nearest(rgb)),
        }
    }

    /// Text in `rgb`.
    pub(crate) fn fg(&self, rgb: Rgb) -> Style {
        Style::new().fg(self.color(rgb))
    }

    /// Text in `rgb`, bold.
    pub(crate) fn bold(&self, rgb: Rgb) -> Style {
        self.fg(rgb).add_modifier(Modifier::BOLD)
    }

    /// The colour of the agent that joined at `at`.
    pub(crate) fn agent(&self, at: usize) -> Rgb {
        self.agents[at % self.agents.len()]
    }
}

/// The nearest of xterm's 256 colours to `rgb`: of its cube of 216 and its
/// 24 greys, the closer.
fn nearest(rgb: Rgb) -> u8 {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let step = |c: u8| -> usize {
        STEPS
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| (i32::from(**s) - i32::from(c)).abs())
            .map_or(0, |(at, _)| at)
    };
    let (r, g, b) = (step(rgb.0), step(rgb.1), step(rgb.2));
    let cube = Rgb(STEPS[r], STEPS[g], STEPS[b]);
    let mean = (u32::from(rgb.0) + u32::from(rgb.1) + u32::from(rgb.2)) / 3;
    let grey_at = ((mean.saturating_sub(8)) / 10).min(23) as u8;
    let grey_level = 8 + 10 * grey_at;
    let grey = Rgb(grey_level, grey_level, grey_level);
    let apart = |a: Rgb| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, rgb.0) + d(a.1, rgb.1) + d(a.2, rgb.2)
    };
    if apart(grey) < apart(cube) {
        232 + grey_at
    } else {
        16 + 36 * r as u8 + 6 * g as u8 + b as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_fall_on_the_nearest_of_the_256_without_24_bits() {
        assert_eq!(nearest(Rgb(0, 0, 0)), 16);
        assert_eq!(nearest(Rgb(255, 255, 255)), 231);
        assert_eq!(nearest(Rgb(128, 128, 128)), 244);
        assert_eq!(nearest(Rgb(200, 164, 106)), 179);
        let p = Palette::new(Theme::Graphite, Depth::Indexed);
        assert_eq!(p.color(p.text), Color::Indexed(nearest(p.text)));
        assert_eq!(Rgb(0, 0, 0).mix(Rgb(200, 100, 50), 0.5), Rgb(100, 50, 25));
    }
}
