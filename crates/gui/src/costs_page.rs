//! Where the money goes: what each payer spent over the period, summed as
//! time went, then by model. What a subscription paid for is valued at the
//! API's list prices: an estimate, said so, never a bill.

use bevy_egui::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use iq_core::{Agent, Usd};
use iq_ui::costs::{Payer, Total};
use iq_ui::{Board, PERIODS};

use crate::theme::{ACCENT, CYAN, DIM, EDGE, GREEN, RAISED, TEXT};

/// Each payer's colour.
fn colour(payer: Payer) -> Color32 {
    match payer {
        Payer::Provider => GREEN,
        Payer::Subscription(Agent::ClaudeCode) => ACCENT,
        Payer::Subscription(Agent::Codex) => CYAN,
    }
}

/// A period's name: `1h`, `24h`, `7d`.
fn period_name(seconds: u64) -> String {
    if seconds >= 2 * 24 * 3600 {
        format!("{}d", seconds / (24 * 3600))
    } else {
        format!("{}h", seconds / 3600)
    }
}

/// Draws the page. Returns the period the person chose, if they chose one.
pub(crate) fn show(ui: &mut Ui, board: &Board) -> Option<u64> {
    let now = iq_ui::now();
    let since = now.saturating_sub(board.period());
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Costs").size(20.0).strong().color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for period in PERIODS.iter().rev() {
                let on = board.period() == *period;
                if ui.selectable_label(on, period_name(*period)).clicked() {
                    chosen = Some(*period);
                }
            }
        });
    });
    ui.label(
        RichText::new(
            "What a subscription paid for is valued at the API's list prices: what it would \
             have cost without one, not a bill.",
        )
        .small()
        .color(DIM),
    );
    ui.add_space(8.0);
    if !board.was_read() {
        ui.label(RichText::new("The agents' calls are being read…").color(DIM));
        return chosen;
    }
    let totals = board.costs().totals(since);
    if totals.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(format!(
                    "No call to a model in the last {}.",
                    period_name(board.period())
                ))
                .color(DIM),
            );
        });
        return chosen;
    }
    ui.horizontal_wrapped(|ui| {
        for payer in payers(&totals) {
            payer_card(ui, payer, &totals);
        }
    });
    ui.add_space(10.0);
    running_plot(ui, board, since, now);
    ui.add_space(10.0);
    ui.label(RichText::new("By model").strong().color(TEXT));
    ui.add_space(2.0);
    egui::Grid::new("costs-by-model")
        .striped(true)
        .num_columns(5)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            for title in ["Paid by", "Model", "Calls", "Cost", ""] {
                ui.label(RichText::new(title).small().color(DIM));
            }
            ui.end_row();
            for total in &totals {
                ui.label(RichText::new(total.payer.name()).color(colour(total.payer)));
                ui.label(RichText::new(&total.model).monospace());
                ui.label(total.calls.to_string());
                ui.label(RichText::new(cost_text(total.payer, total.cost)).strong());
                if total.unpriced > 0 {
                    ui.label(
                        RichText::new(format!("{} without a known price", total.unpriced))
                            .small()
                            .color(DIM),
                    );
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    chosen
}

/// The payers among `totals`, in a fixed order.
fn payers(totals: &[Total]) -> Vec<Payer> {
    let mut payers: Vec<Payer> = totals.iter().map(|t| t.payer).collect();
    payers.sort();
    payers.dedup();
    payers
}

/// A cost as the page writes it: estimates after `~`.
fn cost_text(payer: Payer, cost: Usd) -> String {
    match payer {
        Payer::Provider => cost.to_string(),
        Payer::Subscription(_) => format!("~{cost}"),
    }
}

/// One payer's sum over the period, in large.
fn payer_card(ui: &mut Ui, payer: Payer, totals: &[Total]) {
    let mine: Vec<&Total> = totals.iter().filter(|t| t.payer == payer).collect();
    let mut cost = Usd(0.0);
    for total in &mine {
        cost += total.cost;
    }
    let calls: usize = mine.iter().map(|t| t.calls).sum();
    Frame::new()
        .fill(RAISED)
        .stroke(egui::Stroke::new(1.0, EDGE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_min_width(200.0);
            ui.vertical(|ui| payer_lines(ui, payer, cost, calls));
        });
}

/// What a payer's card says: who, how much, and how it is counted.
fn payer_lines(ui: &mut Ui, payer: Payer, cost: Usd, calls: usize) {
    ui.label(RichText::new(payer.name()).color(colour(payer)).strong());
    ui.label(
        RichText::new(cost_text(payer, cost))
            .size(28.0)
            .strong()
            .color(TEXT),
    );
    let what = match payer {
        Payer::Provider => "billed",
        Payer::Subscription(_) => "at list prices",
    };
    ui.label(
        RichText::new(format!("{calls} calls, {what}"))
            .small()
            .color(DIM),
    );
}

/// Each payer's spending summed as time went, hours before now along the
/// bottom.
fn running_plot(ui: &mut Ui, board: &Board, since: u64, now: u64) {
    let lines = board.costs().running(since);
    Plot::new("costs-running")
        .height(220.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .legend(Legend::default())
        .x_axis_label("hours before now")
        .y_axis_label("dollars")
        .include_x(-(board.period() as f64) / 3600.0)
        .include_x(0.0)
        .include_y(0.0)
        .show(ui, |plot| {
            for (payer, points) in lines {
                let mut points: Vec<[f64; 2]> = points
                    .iter()
                    .map(|(at, sum)| [-(now.saturating_sub(*at) as f64) / 3600.0, *sum])
                    .collect();
                // The line goes on to now, flat since the last call.
                if let Some(&[_, last]) = points.last() {
                    points.push([0.0, last]);
                }
                plot.line(
                    Line::new(payer.name(), PlotPoints::from(points))
                        .color(colour(payer))
                        .width(2.0),
                );
            }
        });
}
