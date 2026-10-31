//! What moves in the live part: the spinner of an agent at work, a light
//! running along the words that say it works, a message's spark running
//! along the status line from its writer to its reader, whose mark then
//! glows. Nothing moves while nothing happens, and the terminal is then
//! drawn only when something changes. Pages open at once: an effect in
//! the way of what the person asked for only makes it feel slow.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use iq_ui::Post;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::theme::{Palette, Rgb};

/// The spinner's frames.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How long a spinner's frame shows.
const SPIN: Duration = Duration::from_millis(80);

/// How long the light takes to run along the words, in seconds.
const SHIMMER: f32 = 2.2;

/// How long a message's spark runs.
const FLIGHT: Duration = Duration::from_millis(900);

/// How long a mark glows once a spark lands.
const GLOW: Duration = Duration::from_millis(700);

/// Where a spark runs from or to: the person, or an agent.
pub(crate) const YOU: &str = "you";

/// A message's spark on its way.
#[derive(Debug, Clone)]
struct Flight {
    from: String,
    to: String,
    since: Instant,
}

/// What moves.
pub(crate) struct Motion {
    started: Instant,
    flights: Vec<Flight>,
    /// When each mark last had a spark land on it.
    landed: HashMap<String, Instant>,
}

impl Default for Motion {
    fn default() -> Self {
        let now = Instant::now();
        Motion {
            started: now,
            flights: Vec::new(),
            landed: HashMap::new(),
        }
    }
}

impl Motion {
    /// Seconds since iq started, for what turns or pulses.
    fn time(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    /// The spinner's frame now.
    pub(crate) fn spinner(&self) -> &'static str {
        let at = self.started.elapsed().as_millis() / SPIN.as_millis();
        SPINNER[(at as usize) % SPINNER.len()]
    }

    /// How bright a pulsing mark is now, from 0 to 1.
    pub(crate) fn pulse(&self, speed: f32) -> f32 {
        0.5 + 0.5 * (self.time() * speed).sin()
    }

    /// `text` with a light running along it: `base`, lit toward `light`
    /// where the light passes.
    pub(crate) fn shimmer(
        &self,
        text: &str,
        base: Rgb,
        light: Rgb,
        p: &Palette,
    ) -> Vec<Span<'static>> {
        let count = text.chars().count().max(1) as f32;
        // The light goes a little past each end, so that it leaves.
        let at = (self.time() % SHIMMER) / SHIMMER * (count + 8.0) - 4.0;
        text.chars()
            .enumerate()
            .map(|(i, c)| {
                let near = (1.0 - ((i as f32 - at).abs() / 3.0)).max(0.0);
                Span::styled(c.to_string(), p.fg(base.mix(light, near)))
            })
            .collect()
    }

    /// A message posted: its spark starts from its writer.
    pub(crate) fn posted(&mut self, post: &Post) {
        let mark = |name: &str| {
            if name == iq_agent::PERSON {
                YOU.to_owned()
            } else {
                name.to_owned()
            }
        };
        self.flights.push(Flight {
            from: mark(&post.from),
            to: mark(&post.to),
            since: Instant::now(),
        });
    }

    /// How a mark glows now, from 0 to 1: once a spark landed on it.
    pub(crate) fn glow(&self, mark: &str) -> f32 {
        self.landed.get(mark).map_or(0.0, |at| {
            (1.0 - at.elapsed().as_secs_f32() / GLOW.as_secs_f32()).max(0.0)
        })
    }

    /// Runs the sparks along `row`, the status line in `buf`, between the
    /// marks drawn there, by their columns in `marks`.
    pub(crate) fn fly(
        &mut self,
        buf: &mut Buffer,
        row: Rect,
        marks: &HashMap<String, u16>,
        p: &Palette,
    ) {
        let mut landed = Vec::new();
        self.flights.retain(|flight| {
            let (Some(from), Some(to)) = (marks.get(&flight.from), marks.get(&flight.to)) else {
                // Toward a mark not drawn, as to every agent: it lands at once.
                landed.push(flight.to.clone());
                return false;
            };
            let t = flight.since.elapsed().as_secs_f32() / FLIGHT.as_secs_f32();
            if t >= 1.0 {
                landed.push(flight.to.clone());
                return false;
            }
            let eased = t * t * (3.0 - 2.0 * t);
            let head = f32::from(*from) + (f32::from(*to) - f32::from(*from)) * eased;
            let going = if to >= from { 1.0 } else { -1.0 };
            for back in 0..6 {
                // Rounded before it is bounded: a hair inside the edge
                // rounds onto it.
                let x = (head - going * back as f32).round();
                if x < f32::from(row.left()) || x >= f32::from(row.right()) {
                    continue;
                }
                let lit = 1.0 - back as f32 / 6.0;
                let cell = &mut buf[(x as u16, row.top())];
                let tint = p.faint.mix(p.accent, lit);
                if back == 0 {
                    cell.set_symbol("●");
                }
                cell.set_style(Style::new().fg(p.color(tint)));
            }
            true
        });
        let now = Instant::now();
        for mark in landed {
            self.landed.insert(mark, now);
        }
        self.landed.retain(|_, at| at.elapsed() < GLOW);
    }

    /// Whether anything moves: the terminal is then drawn often.
    pub(crate) fn moving(&self) -> bool {
        !self.flights.is_empty() || !self.landed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_ui::look::Theme;

    #[test]
    fn a_spark_leaves_its_writer_and_lands_on_its_reader() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let mut motion = Motion::default();
        motion.posted(&Post {
            from: "lead".into(),
            to: iq_agent::PERSON.into(),
            text: "done".into(),
        });
        let row = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(row);
        let marks: HashMap<String, u16> = [("lead".to_owned(), 20), (YOU.to_owned(), 4)]
            .into_iter()
            .collect();
        motion.fly(&mut buf, row, &marks, &p);
        assert_eq!(buf[(20, 0)].symbol(), "●");
        assert!(motion.moving());
        // Toward a mark not drawn, it lands at once, and the mark glows.
        motion.posted(&Post {
            from: "lead".into(),
            to: "gone".into(),
            text: "?".into(),
        });
        motion.fly(&mut buf, row, &marks, &p);
        assert!(motion.glow("gone") > 0.9);
        assert_eq!(motion.glow("lead"), 0.0);
    }

    #[test]
    fn a_spark_from_the_last_column_stays_within_its_row() {
        let p = Palette::new(Theme::Ink, Depth::True);
        let row = Rect::new(0, 0, 30, 1);
        let marks: HashMap<String, u16> = [("lead".to_owned(), 29), (YOU.to_owned(), 4)]
            .into_iter()
            .collect();
        // Its trail runs behind it, toward the edge it left, at every
        // moment of its flight.
        for ms in (0..FLIGHT.as_millis() as u64).step_by(5) {
            let mut motion = Motion::default();
            motion.flights.push(Flight {
                from: "lead".into(),
                to: YOU.into(),
                since: Instant::now() - Duration::from_millis(ms),
            });
            let mut buf = Buffer::empty(row);
            motion.fly(&mut buf, row, &marks, &p);
        }
    }
}
