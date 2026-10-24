//! Where the money goes: what the provider billed over the period, summed
//! as time went, and how much of each subscription's windows the calls
//! took, since a subscription is paid by the month; then each model's
//! calls. Drawn to fit a pane of any width, or the column on the right.

use bevy_egui::egui::{self, CornerRadius, Frame, Margin, RichText, Ui};
use egui_plot::{Line, Plot, PlotPoints};
use iq_core::{TokenCount, Usd};
use iq_ui::costs::{Payer, Total};
use iq_ui::subscriptions::{Subscriptions, WindowView, percent, span};
use iq_ui::{Board, PERIODS};

use crate::theme::{self, Palette};

/// A period's name: `1h`, `24h`, `7d`.
fn period_name(seconds: u64) -> String {
    if seconds >= 2 * 24 * 3600 {
        format!("{}d", seconds / (24 * 3600))
    } else {
        format!("{}h", seconds / 3600)
    }
}

/// Draws the costs, `titled` unless the pane's title says it already:
/// iq's calls, or the person's own sessions too, over the period they
/// choose.
pub(crate) fn show(ui: &mut Ui, board: &mut Board, titled: bool, p: &Palette) {
    let now = iq_ui::now();
    let since = now.saturating_sub(board.period());
    let mut chosen = None;
    let mut everyone = board.everyone();
    ui.horizontal(|ui| {
        if titled {
            ui.label(RichText::new("Costs").font(theme::bold(15.0)).color(p.text));
        } else {
            ui.label(RichText::new("Over the last").small().color(p.dim));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for period in PERIODS.iter().rev() {
                let on = board.period() == *period;
                let text = RichText::new(period_name(*period))
                    .font(theme::mono(11.5))
                    .color(if on { p.text } else { p.dim });
                if ui.selectable_label(on, text).clicked() {
                    chosen = Some(*period);
                }
            }
        });
    });
    ui.horizontal(|ui| {
        let choice = |on: bool, text: &str| {
            RichText::new(text)
                .small()
                .color(if on { p.text } else { p.dim })
        };
        if ui
            .selectable_label(!everyone, choice(!everyone, "By iq"))
            .on_hover_text("What iq's agents spent")
            .clicked()
        {
            everyone = false;
        }
        if ui
            .selectable_label(everyone, choice(everyone, "With your own sessions"))
            .on_hover_text(
                "Also what you spent yourself in Claude Code or Codex, outside iq: their files \
                 keep every call, made by iq or not",
            )
            .clicked()
        {
            everyone = true;
        }
    });
    if let Some(period) = chosen {
        board.sum_over(period);
    }
    board.count_everyone(everyone);
    ui.add_space(4.0);
    crate::steward_view::show(ui, board, p);
    if !board.was_read() {
        ui.label(
            RichText::new("The agents' calls are being read…")
                .small()
                .color(p.dim),
        );
        return;
    }
    let totals = board.costs().totals(since, everyone);
    if totals.is_empty() {
        ui.add_space(12.0);
        let who = if everyone {
            "No call"
        } else {
            "iq made no call"
        };
        ui.label(
            RichText::new(format!(
                "{who} to a model in the last {}.",
                period_name(board.period())
            ))
            .color(p.dim),
        );
        return;
    }
    ui.add_space(4.0);
    let windows = board.subscriptions().windows(now);
    let iq_only = (!everyone).then(|| board.subscriptions());
    ui.horizontal_wrapped(|ui| {
        for payer in payers(&totals) {
            payer_card(ui, payer, &totals, &windows, iq_only, p);
        }
    });
    // Only what is billed is summed in dollars as time goes.
    if totals.iter().any(|t| t.payer == Payer::Provider) {
        ui.add_space(6.0);
        running_plot(ui, board, since, now, p);
    }
    ui.add_space(6.0);
    ui.label(RichText::new("By model").small().color(p.dim));
    egui::Grid::new("costs-by-model")
        .num_columns(4)
        .spacing([14.0, 3.0])
        .show(ui, |ui| {
            for total in &totals {
                ui.label(
                    RichText::new(total.payer.name())
                        .small()
                        .color(theme::payer(p, total.payer)),
                );
                ui.label(RichText::new(&total.model).font(theme::mono(11.5)));
                ui.label(
                    RichText::new(format!("{} calls", total.calls))
                        .small()
                        .color(p.dim),
                );
                match total.payer {
                    Payer::Provider => {
                        let mut cost =
                            RichText::new(total.cost.to_string()).font(theme::mono_medium(12.0));
                        if total.unpriced > 0 {
                            cost = cost.color(p.warn);
                        }
                        let label = ui.label(cost);
                        if total.unpriced > 0 {
                            label
                                .on_hover_text(format!("{} without a known price", total.unpriced));
                        }
                    }
                    Payer::Subscription(_) => {
                        ui.label(
                            RichText::new(format!("{} tokens", TokenCount(total.tokens)))
                                .font(theme::mono(11.5))
                                .color(p.dim),
                        );
                    }
                }
                ui.end_row();
            }
        });
    ui.label(
        RichText::new(
            "A subscription is paid by the month: what counts is how much of its windows the \
             calls take, not dollars.",
        )
        .small()
        .color(p.dim),
    );
    by_kind(ui, board, since, p);
}

/// What each kind of task iq's agents did spent over the period, as the
/// assay's rules place them: the kinds are the person's, written with
/// its judge.
fn by_kind(ui: &mut Ui, board: &Board, since: u64, p: &Palette) {
    let kinds = board.costs().by_kind(since);
    if kinds.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label(RichText::new("By kind of task").small().color(p.dim));
    if kinds.iter().all(|k| k.kind.is_none()) {
        ui.label(
            RichText::new(
                "No kind of task is written yet: the assay's judge proposes them with you (Ctrl-L).",
            )
            .small()
            .color(p.dim),
        );
    }
    let most = kinds
        .iter()
        .map(|k| k.billed.0 + k.listed.0)
        .fold(0.0, f64::max);
    egui::Grid::new("costs-by-kind")
        .num_columns(5)
        .spacing([14.0, 3.0])
        .show(ui, |ui| {
            for kind in &kinds {
                let name = kind.kind.as_deref().unwrap_or("no kind");
                ui.label(RichText::new(name).small().color(if kind.kind.is_some() {
                    p.text
                } else {
                    p.dim
                }));
                ui.label(
                    RichText::new(format!("{} tasks", kind.tasks))
                        .small()
                        .color(p.dim),
                );
                ui.label(RichText::new(kind.billed.to_string()).font(theme::mono_medium(12.0)))
                    .on_hover_text("billed by the provider");
                ui.label(
                    RichText::new(format!("{} listed", kind.listed))
                        .font(theme::mono(11.5))
                        .color(p.dim),
                )
                .on_hover_text("what the subscriptions' calls would cost at list prices");
                let share = if most > 0.0 {
                    ((kind.billed.0 + kind.listed.0) / most) as f32
                } else {
                    0.0
                };
                let (bar, _) = ui.allocate_exact_size(egui::vec2(80.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(bar, 3.0, p.track);
                let mut filled = bar;
                filled.set_width(bar.width() * share.clamp(0.0, 1.0));
                ui.painter().rect_filled(filled, 3.0, p.accent);
                ui.end_row();
            }
        });
}

/// The payers among `totals`, in a fixed order.
fn payers(totals: &[Total]) -> Vec<Payer> {
    let mut payers: Vec<Payer> = totals.iter().map(|t| t.payer).collect();
    payers.sort();
    payers.dedup();
    payers
}

/// One payer's period, in large: the provider's bill, or how much of a
/// subscription's windows are used, as its agent last reported them; iq's
/// part of them alone when `iq_only` holds the subscriptions to tell it.
fn payer_card(
    ui: &mut Ui,
    payer: Payer,
    totals: &[Total],
    windows: &[WindowView],
    iq_only: Option<&Subscriptions>,
    p: &Palette,
) {
    let mine: Vec<&Total> = totals.iter().filter(|t| t.payer == payer).collect();
    let calls: usize = mine.iter().map(|t| t.calls).sum();
    let tokens: u64 = mine.iter().map(|t| t.tokens).sum();
    Frame::new()
        .fill(p.card)
        .stroke(egui::Stroke::new(1.0, p.edge))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_width(150.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(payer.name()).small().color(p.dim));
                let big = |text: String| {
                    RichText::new(text)
                        .font(theme::mono_medium(21.0))
                        .color(theme::payer(p, payer))
                };
                match payer {
                    Payer::Provider => {
                        let mut cost = Usd(0.0);
                        for total in &mine {
                            cost += total.cost;
                        }
                        ui.label(big(cost.to_string()));
                        ui.label(
                            RichText::new(format!("billed · {calls} calls"))
                                .small()
                                .color(p.dim),
                        );
                    }
                    Payer::Subscription(agent) => {
                        let mut shares = windows.iter().filter(|w| w.agent == agent);
                        let part = |w: &WindowView| iq_only.and_then(|s| s.iq_part(w));
                        match shares.next() {
                            Some(first) => {
                                ui.label(big(window_share(first, part(first))));
                                for other in shares {
                                    ui.label(
                                        RichText::new(window_share(other, part(other)))
                                            .font(theme::mono(12.0))
                                            .color(p.text),
                                    );
                                }
                                // Until iq's rates are learnt, only the whole
                                // window is known.
                                if iq_only.is_some() && part(first).is_none() {
                                    ui.label(
                                        RichText::new("the whole window, your sessions too")
                                            .small()
                                            .color(p.dim),
                                    );
                                }
                            }
                            None => {
                                ui.label(big("subscription".to_owned()));
                            }
                        }
                        ui.label(
                            RichText::new(format!("{calls} calls · {} tokens", TokenCount(tokens)))
                                .small()
                                .color(p.dim),
                        );
                    }
                }
            });
        });
}

/// How much of a window is used, `42% of 5h`: iq's `part` of it when
/// given, else all of it.
fn window_share(window: &WindowView, part: Option<f64>) -> String {
    let used = match (part, window.estimate) {
        (Some(part), _) => format!("~{}", percent(part)),
        (None, Some(estimate)) => format!("~{}", percent(estimate)),
        (None, None) => percent(window.reported),
    };
    format!("{used} of {}", span(window.minutes))
}

/// What the provider billed, summed as time went, hours before now along
/// the bottom.
fn running_plot(ui: &mut Ui, board: &Board, since: u64, now: u64, p: &Palette) {
    let lines = board.costs().running(since, board.everyone());
    Plot::new("costs-running")
        .height(110.0)
        .show_background(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_grid(false)
        .include_x(-(board.period() as f64) / 3600.0)
        .include_x(0.0)
        .include_y(0.0)
        .x_axis_formatter(|mark, _| {
            if mark.value.abs() < 0.01 {
                "now".to_owned()
            } else {
                format!("{:.0}h ago", -mark.value)
            }
        })
        .y_axis_formatter(|mark, _| format!("${:.2}", mark.value))
        .show(ui, |plot| {
            for (payer, points) in lines {
                if payer != Payer::Provider {
                    continue;
                }
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
                        .color(theme::payer(p, payer))
                        .fill(0.0)
                        .fill_alpha(0.12)
                        .width(1.6),
                );
            }
        });
}
