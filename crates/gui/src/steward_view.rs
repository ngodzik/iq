//! The steward as the costs show it: each limit a dial, what iq used lit
//! in its payer's colour, what was spent outside iq beside it in grey, a
//! mark where an even pace would be now; the dial glows as the pace gets
//! too fast. Then what the steward and the agents said to each other.

use bevy_egui::egui::{self, Color32, Pos2, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};
use iq_ui::Board;
use iq_ui::steward::{Gauge, Level};
use iq_ui::subscriptions::local_time;

use crate::theme::{self, Palette};

/// A dial's size.
const DIAL: f32 = 84.0;

/// Where a dial's arc starts and how far it turns: three quarters of a
/// turn, open at the bottom.
const FROM: f32 = 0.75 * std::f32::consts::PI;
const TURN: f32 = 1.5 * std::f32::consts::PI;

/// How many letters show.
const LETTERS: usize = 6;

/// The points of an arc around `center`, from `start` to `end` of the
/// dial's turn, each from 0 to 1.
fn arc(center: Pos2, radius: f32, start: f32, end: f32) -> Vec<Pos2> {
    let (start, end) = (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0));
    let steps = ((end - start) * 48.0).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|i| {
            let share = start + (end - start) * i as f32 / steps as f32;
            let angle = FROM + TURN * share;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// A level's colour.
fn colour(level: Level, p: &Palette) -> Color32 {
    match level {
        Level::Calm => p.ok,
        Level::Watch => p.warn,
        Level::Alert | Level::Reached => p.bad,
    }
}

/// Draws the steward's part of the costs: the dials of the limits, and its
/// letters, when there are any.
pub(crate) fn show(ui: &mut Ui, board: &Board, p: &Palette) {
    let gauges = board.gauges();
    if gauges.is_empty() && board.letters().is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Limits")
                .font(theme::bold(15.0))
                .color(p.text),
        );
        let (word, tint) = if board.steward().on() {
            ("the steward writes to who spends too fast", p.dim)
        } else {
            ("the steward only watches", p.dim)
        };
        ui.label(RichText::new(word).small().color(tint));
    });
    ui.horizontal_wrapped(|ui| {
        for gauge in gauges {
            dial(ui, gauge, p);
        }
    });
    let letters = board.letters();
    if !letters.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new("The steward and the crew")
                .small()
                .color(p.dim),
        );
        for letter in letters.iter().rev().take(LETTERS) {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("{} → {}", letter.from, letter.to))
                        .font(theme::mono(11.0))
                        .color(p.info),
                );
                ui.label(RichText::new(&letter.text).small().color(p.text));
            });
        }
    }
    ui.add_space(10.0);
}

/// One limit as a dial, with what it says below.
fn dial(ui: &mut Ui, gauge: &Gauge, p: &Palette) {
    ui.vertical(|ui| {
        ui.set_width(DIAL + 54.0);
        let (rect, response) = ui.allocate_exact_size(vec2(DIAL + 54.0, DIAL), Sense::hover());
        let center = pos2(rect.center().x, rect.top() + DIAL / 2.0 + 4.0);
        let radius = DIAL / 2.0 - 8.0;
        let painter = ui.painter();
        let time = ui.input(|i| i.time) as f32;
        let tint = colour(gauge.level, p);
        // Glowing as the pace gets too fast, pulsing when the limit is near.
        let glow = match gauge.level {
            Level::Calm => 0.0,
            Level::Watch => 0.35,
            Level::Alert => 0.55 + 0.35 * (time * 3.0).sin(),
            Level::Reached => 0.8,
        };
        if glow > 0.0 {
            for (wide, share) in [(14.0, 0.10), (9.0, 0.18)] {
                painter.add(Shape::line(
                    arc(center, radius, 0.0, 1.0),
                    Stroke::new(wide, theme::alpha(tint, share * glow)),
                ));
            }
        }
        painter.add(Shape::line(
            arc(center, radius, 0.0, 1.0),
            Stroke::new(6.0, p.track),
        ));
        let limit = gauge.limit.max(f64::EPSILON);
        let mine = ((gauge.used - gauge.external) / limit) as f32;
        let all = (gauge.used / limit) as f32;
        if all > 0.0 {
            painter.add(Shape::line(
                arc(center, radius, 0.0, all),
                Stroke::new(6.0, theme::alpha(p.dim, 0.7)),
            ));
        }
        if mine > 0.0 {
            painter.add(Shape::line(
                arc(center, radius, 0.0, mine),
                Stroke::new(6.0, theme::payer(p, gauge.payer)),
            ));
        }
        // Where an even pace would be now.
        let even = arc(center, radius, gauge.elapsed as f32, gauge.elapsed as f32);
        if let Some(at) = even.first() {
            let out = (*at - center).normalized();
            painter.line_segment([*at - out * 7.0, *at + out * 7.0], Stroke::new(1.5, p.text));
        }
        painter.text(
            center - vec2(0.0, 4.0),
            egui::Align2::CENTER_CENTER,
            gauge.amount(gauge.used),
            theme::mono_medium(14.0),
            p.text,
        );
        painter.text(
            center + vec2(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            format!("of {}", gauge.amount(gauge.limit)),
            theme::mono(10.5),
            p.dim,
        );
        response.on_hover_text(gauge.said());
        ui.label(RichText::new(gauge.name()).small().color(p.text));
        let below = match (gauge.level, gauge.full_at) {
            (Level::Reached, _) => "reached: closed".to_owned(),
            (Level::Calm, _) => "calm".to_owned(),
            (_, Some(at)) => format!("full {}", local_time(at)),
            (_, None) => "ahead of pace".to_owned(),
        };
        ui.label(RichText::new(below).small().color(tint));
        if gauge.external > 0.05 {
            ui.label(
                RichText::new(format!("{} external", gauge.amount(gauge.external)))
                    .small()
                    .color(p.dim),
            );
        }
    });
}
