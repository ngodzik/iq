//! What iq's work went to: over a day, a week, a month, a year or all of
//! it, back as far as the person goes, summed by category, model, project
//! or agent; a chart part by part, each group stacked, a table of the
//! groups, and the dearest tasks. Drawn to fit a pane of any width.

use bevy_egui::egui::{self, Color32, RichText, Ui};
use egui_plot::{Bar, BarChart, Legend, Plot};
use iq_core::{TokenCount, Usd};
use iq_ui::Board;
use iq_ui::observations::{By, Group, Lens, Seen, Span};

use crate::theme::{self, Palette};

/// How many groups the chart shows apart; the rest are summed as one.
const CHARTED: usize = 6;

/// Draws the observations, and takes what the person chose.
pub(crate) fn show(ui: &mut Ui, board: &mut Board, p: &Palette) {
    let mut lens = board.lens();
    controls(ui, &mut lens, p);
    if lens != board.lens() {
        board.look_at(lens);
    }
    let seen = board.seen();
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(&seen.title)
                .font(theme::bold(15.0))
                .color(p.text),
        );
        summary(ui, &seen, p);
    });
    if seen.total.tasks == 0 {
        ui.add_space(8.0);
        ui.label(RichText::new("No task of iq's in this period.").color(p.dim));
        return;
    }
    if seen.unplaced && lens.by == By::Category {
        ui.label(
            RichText::new(
                "No task is in a category: name yours in ~/.iq/settings.toml, [[observe.category]], \
                 and a cheap model places the work in them as it ends.",
            )
            .small()
            .color(p.warn),
        );
    }
    ui.add_space(6.0);
    chart(ui, &seen, p);
    ui.add_space(6.0);
    table(ui, &seen, p);
    ui.add_space(10.0);
    ui.label(
        RichText::new("The dearest tasks")
            .small()
            .strong()
            .color(p.dim),
    );
    for task in &seen.top {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(
                RichText::new((task.billed + task.listed).to_string())
                    .font(theme::mono_medium(11.5))
                    .color(p.accent),
            );
            ui.label(
                RichText::new(
                    task.kind
                        .as_deref()
                        .unwrap_or(iq_ui::observations::UNPLACED),
                )
                .small()
                .color(p.info),
            );
            ui.label(
                RichText::new(&task.model)
                    .font(theme::mono(11.0))
                    .color(p.dim),
            );
            if !task.ok {
                ui.label(RichText::new("failed").small().color(p.bad));
            }
            let prompt: String = task.prompt.split_whitespace().collect::<Vec<_>>().join(" ");
            ui.add(egui::Label::new(RichText::new(prompt).small().color(p.text)).truncate());
        });
    }
}

/// The span, how far back, and what the tasks are summed by.
fn controls(ui: &mut Ui, lens: &mut Lens, p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        for span in Span::ALL {
            let on = lens.span == span;
            let text = RichText::new(span.name())
                .font(theme::mono(11.5))
                .color(if on { p.text } else { p.dim });
            if ui.selectable_label(on, text).clicked() && !on {
                *lens = Lens {
                    span,
                    back: 0,
                    by: lens.by,
                };
            }
        }
        if lens.span != Span::All {
            ui.add_space(10.0);
            if ui
                .button(RichText::new("‹").color(p.text))
                .on_hover_text("the period before")
                .clicked()
            {
                lens.back += 1;
            }
            let later = ui.add_enabled(
                lens.back > 0,
                egui::Button::new(RichText::new("›").color(p.text)),
            );
            if later.on_hover_text("the period after").clicked() {
                lens.back = lens.back.saturating_sub(1);
            }
            if lens.back > 0 && ui.button(RichText::new("now").small()).clicked() {
                lens.back = 0;
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("By").small().color(p.dim));
        for by in By::ALL {
            let on = lens.by == by;
            let text = RichText::new(by.name())
                .small()
                .color(if on { p.text } else { p.dim });
            if ui.selectable_label(on, text).clicked() {
                lens.by = by;
            }
        }
    });
}

/// The period in a line: its tasks, what they cost, how long they took,
/// and against the period before.
fn summary(ui: &mut Ui, seen: &Seen, p: &Palette) {
    let total = &seen.total;
    let mut said = format!(
        "{} task{} · {} billed · {} at list prices · {}",
        total.tasks,
        if total.tasks == 1 { "" } else { "s" },
        Usd(total.billed),
        Usd(total.listed),
        duration(total.seconds)
    );
    if total.failed > 0 {
        said.push_str(&format!(" · {} failed", total.failed));
    }
    ui.label(RichText::new(said).small().color(p.dim));
    if let Some(before) = seen.before.filter(|b| *b > 0.0) {
        let change = (total.cost() - before) / before * 100.0;
        let colour = if change > 0.0 { p.warn } else { p.ok };
        ui.label(
            RichText::new(format!(
                "{change:+.0}% against the period before ({})",
                Usd(before)
            ))
            .small()
            .color(colour),
        );
    }
}

/// A colour for the group at `at`, in the theme.
fn colour(p: &Palette, at: usize) -> Color32 {
    // Neighbours apart in hue, the theme's warm ones not side by side.
    let colours = [p.accent, p.info, p.ok, p.bad, p.you, p.warn, p.idle];
    colours[at % colours.len()]
}

/// What each group spent part by part, stacked.
fn chart(ui: &mut Ui, seen: &Seen, p: &Palette) {
    let charted = seen.groups.len().min(CHARTED);
    let mut charts: Vec<BarChart> = Vec::new();
    for at in 0..=charted {
        let rest = at == charted;
        if rest && seen.groups.len() <= CHARTED {
            break;
        }
        let name = if rest {
            "the rest".to_owned()
        } else {
            seen.groups[at].name.clone()
        };
        let bars: Vec<Bar> = seen
            .slices
            .iter()
            .enumerate()
            .map(|(x, slice)| {
                let value = if rest {
                    slice.costs.iter().skip(CHARTED).sum()
                } else {
                    slice.costs[at]
                };
                Bar::new(x as f64, value)
                    .width(0.7)
                    .name(&slice.name)
                    .fill(theme::alpha(colour(p, at), 0.7))
                    .stroke(egui::Stroke::new(1.0, colour(p, at)))
            })
            .collect();
        let mut chart = BarChart::new(name, bars).color(theme::alpha(colour(p, at), 0.85));
        let below: Vec<&BarChart> = charts.iter().collect();
        if !below.is_empty() {
            chart = chart.stack_on(&below);
        }
        charts.push(chart);
    }
    let names: Vec<String> = seen.slices.iter().map(|s| s.name.clone()).collect();
    Plot::new(("observations", seen.from))
        .height(150.0)
        .show_background(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_grid(false)
        .include_y(0.0)
        .legend(Legend::default())
        .x_axis_formatter(move |mark, _| {
            let at = mark.value.round();
            if (mark.value - at).abs() > 0.01 || at < 0.0 {
                return String::new();
            }
            names.get(at as usize).cloned().unwrap_or_default()
        })
        .y_axis_formatter(|mark, _| format!("${:.2}", mark.value))
        .show(ui, |plot| {
            for chart in charts {
                plot.bar_chart(chart);
            }
        });
}

/// The groups, the dearest first.
fn table(ui: &mut Ui, seen: &Seen, p: &Palette) {
    let most = seen.groups.first().map_or(0.0, Group::cost);
    egui::Grid::new(("observations-groups", seen.from))
        .num_columns(7)
        .spacing([14.0, 3.0])
        .show(ui, |ui| {
            for head in [
                "",
                "tasks",
                "failed",
                "billed",
                "list prices",
                "tokens",
                "time",
            ] {
                ui.label(RichText::new(head).font(theme::mono(10.5)).color(p.dim));
            }
            ui.label("");
            ui.end_row();
            for (at, group) in seen.groups.iter().enumerate() {
                let mono = |text: String, colour| {
                    RichText::new(text).font(theme::mono(11.5)).color(colour)
                };
                ui.horizontal(|ui| {
                    let (dot, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 12.0), egui::Sense::hover());
                    let tint = if at < CHARTED {
                        colour(p, at)
                    } else {
                        colour(p, CHARTED)
                    };
                    ui.painter().circle_filled(dot.center(), 3.5, tint);
                    ui.label(RichText::new(&group.name).color(p.text));
                });
                ui.label(mono(group.tasks.to_string(), p.text));
                ui.label(mono(
                    if group.failed > 0 {
                        group.failed.to_string()
                    } else {
                        "·".into()
                    },
                    if group.failed > 0 { p.bad } else { p.dim },
                ));
                ui.label(mono(Usd(group.billed).to_string(), p.text));
                ui.label(mono(Usd(group.listed).to_string(), p.dim));
                ui.label(mono(TokenCount(group.tokens).to_string(), p.dim));
                ui.label(mono(duration(group.seconds), p.dim));
                let (bar, _) = ui.allocate_exact_size(egui::vec2(70.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(bar, 3.0, p.track);
                let mut filled = bar;
                let share = if most > 0.0 {
                    (group.cost() / most) as f32
                } else {
                    0.0
                };
                filled.set_width(bar.width() * share.clamp(0.0, 1.0));
                ui.painter().rect_filled(filled, 3.0, p.accent);
                ui.end_row();
            }
        });
}

/// A length of time, as `2h 05m`, `12m`, `40s`.
fn duration(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}
