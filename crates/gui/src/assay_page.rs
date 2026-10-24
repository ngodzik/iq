//! The assay's page: who judges and what is tested, the runs at work, each
//! kind of task with the cheapest candidate that does it as well as the
//! lead, the exercises, and each one's results, the kinds of candidates
//! apart. Drawn to fit a pane of any width.

use bevy_egui::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use iq_core::TokenCount;
use iq_ui::Board;
use iq_ui::assay::{JUDGE, Kind, Phase, Progress, Row, State, Summary, Table};
use iq_ui::crew::Request;

use crate::theme::{self, Palette};

/// Draws the assay in the pane at `index`; what the person asks of it goes
/// to `asked`.
pub(crate) fn show(
    ui: &mut Ui,
    board: &mut Board,
    index: usize,
    p: &Palette,
    asked: &mut Vec<Request>,
) {
    let view = board.assay().clone();
    let overview = view.overview();
    // The pane's title says it is the assay.
    ui.label(
        RichText::new(
            "The same frozen exercises for every candidate, checked by hidden tests, then judged \
             blind: for each kind of task, the cheapest agent that does it as well as your lead.",
        )
        .small()
        .color(p.dim),
    );
    ui.add_space(6.0);
    if !view.told() {
        ui.label(
            RichText::new("The assay's folder is being read…")
                .small()
                .color(p.dim),
        );
        return;
    }
    if let Some(trouble) = &overview.trouble {
        ui.label(RichText::new(trouble).small().color(p.bad));
    }
    for notice in &overview.notices {
        ui.label(RichText::new(notice).small().color(p.warn));
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let pair = |ui: &mut Ui, name: &str, value: String| {
            ui.label(RichText::new(name).small().color(p.dim));
            ui.label(RichText::new(value).font(theme::mono(11.5)).color(p.text));
        };
        pair(ui, "Judges", overview.judges.join(", "));
        ui.add_space(8.0);
        pair(ui, "Reference", overview.reference.clone());
        ui.add_space(8.0);
        pair(ui, "Tested", overview.allowed.clone());
    });
    ui.label(
        RichText::new(overview.folder.display().to_string())
            .font(theme::mono(11.0))
            .color(p.dim),
    )
    .on_hover_text("The assay's files: the reference, which a copy elsewhere only receives");
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let talking = board.crew().agent(JUDGE).is_some();
        let text = if talking {
            "Open the judge's chat"
        } else {
            "Talk to the judge"
        };
        let button = egui::Button::new(RichText::new(text).color(p.accent).strong())
            .stroke(Stroke::new(1.0, theme::alpha(p.accent, 0.5)));
        if ui
            .add(button)
            .on_hover_text(
                "The judge proposes kinds of task and exercises with you, and asks before \
                 anything that costs",
            )
            .clicked()
            && let Some(request) = board.talk_to_judge(index)
        {
            asked.push(request);
        }
        if !view.running().is_empty()
            && ui
                .button(RichText::new("Stop the runs").color(p.warn))
                .clicked()
        {
            asked.extend(board.stop_runs());
        }
    });
    if !view.running().is_empty() {
        section(ui, "At work", p);
        for progress in view.running() {
            running(ui, progress, p);
        }
    }
    if !overview.categories.is_empty() {
        section(ui, "By kind of task", p);
        for (name, rows, fit) in &overview.categories {
            ui.label(RichText::new(name).font(theme::bold(13.5)).color(p.text));
            match fit {
                Some(fit) => ui.label(RichText::new(fit).small().color(p.ok)),
                None => ui.label(
                    RichText::new(
                        "No candidate billed by the provider does as well as the reference yet.",
                    )
                    .small()
                    .color(p.dim),
                ),
            };
            results(ui, &format!("kind-{name}"), rows, p);
            ui.add_space(6.0);
        }
    }
    section(ui, "Exercises", p);
    if overview.exercises.is_empty() {
        ui.label(
            RichText::new(
                "None yet. Talk to the judge to write the first ones, best from commits of your \
                 own work.",
            )
            .color(p.dim),
        );
    }
    for summary in &overview.exercises {
        exercise(ui, summary, p);
    }
    for table in &overview.tables {
        section(ui, &table_title(table), p);
        results(ui, &format!("table-{}", table.exercise), &table.rows, p);
    }
}

fn section(ui: &mut Ui, title: &str, p: &Palette) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).small().strong().color(p.dim));
    ui.add_space(2.0);
}

fn table_title(table: &Table) -> String {
    let version = table
        .version
        .map(|v| format!(", version {v}"))
        .unwrap_or_default();
    format!("{} · {}{version}", table.exercise, table.title)
}

/// A run at work: its letter and candidate, what it does, what it spent.
fn running(ui: &mut Ui, progress: &Progress, p: &Palette) {
    let (said, colour, glow) = match progress.phase {
        Phase::Waiting => ("waiting", p.idle, 0.0),
        Phase::Working => ("working", p.accent, 0.8),
        Phase::Checking => ("checking", p.info, 0.6),
        Phase::Judging => ("being judged", p.info, 0.6),
        Phase::Done(true) => ("passed", p.ok, 0.0),
        Phase::Done(false) => ("failed", p.bad, 0.0),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
        theme::sphere(ui.painter(), dot.center(), 3.6, colour, glow);
        ui.label(
            RichText::new(&progress.label)
                .font(theme::mono_medium(12.0))
                .color(p.text),
        );
        ui.label(
            RichText::new(&progress.candidate)
                .font(theme::mono(11.5))
                .color(p.text),
        );
        ui.label(RichText::new(said).small().color(colour));
        let elapsed = iq_ui::now().saturating_sub(progress.since);
        ui.label(
            RichText::new(format!(
                "{} tools · {} · {} tokens · {}",
                progress.tools,
                progress.billed,
                TokenCount(progress.tokens),
                minutes(elapsed)
            ))
            .small()
            .color(p.dim),
        );
    });
}

/// An exercise: its name, where it stands, its kinds, its runs, what keeps
/// a draft from being frozen.
fn exercise(ui: &mut Ui, summary: &Summary, p: &Palette) {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.edge))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    RichText::new(&summary.id)
                        .font(theme::mono_medium(12.0))
                        .color(p.text),
                );
                let (state, colour) = match &summary.state {
                    State::Draft => ("draft".to_owned(), p.dim),
                    State::Frozen(v) => (format!("frozen · v{v}"), p.ok),
                    State::Changed(v) => (format!("changed since v{v}"), p.warn),
                    State::Broken(_) => ("broken".to_owned(), p.bad),
                };
                ui.label(RichText::new(state).small().color(colour));
                if !summary.title.is_empty() {
                    ui.label(RichText::new(&summary.title).color(p.text));
                }
                for category in &summary.categories {
                    ui.label(
                        RichText::new(category)
                            .font(theme::mono(10.5))
                            .color(p.info)
                            .background_color(theme::alpha(p.info, 0.10)),
                    );
                }
                ui.label(
                    RichText::new(format!(
                        "{} run{}",
                        summary.runs,
                        if summary.runs == 1 { "" } else { "s" }
                    ))
                    .small()
                    .color(p.dim),
                );
            });
            if let Some(problem) = summary.problems.first() {
                let more = summary.problems.len() - 1;
                let text = if more > 0 {
                    format!("{problem} (and {more} more)")
                } else {
                    problem.clone()
                };
                ui.label(RichText::new(text).small().color(p.warn));
            }
        });
    ui.add_space(4.0);
}

/// Candidates' results, a heading per kind when there are several.
fn results(ui: &mut Ui, id: &str, rows: &[Row], p: &Palette) {
    if rows.is_empty() {
        ui.label(RichText::new("No runs yet.").small().color(p.dim));
        return;
    }
    let kinds: Vec<Kind> = Kind::ALL
        .into_iter()
        .filter(|kind| rows.iter().any(|r| r.kind == *kind))
        .collect();
    for kind in &kinds {
        if kinds.len() > 1 {
            ui.label(RichText::new(kind.plural()).small().color(p.dim));
        }
        egui::Grid::new((id, kind.plural()))
            .num_columns(7)
            .spacing([14.0, 3.0])
            .striped(false)
            .show(ui, |ui| {
                for head in ["", "passed", "tests", "score", "a run", "a success", "time"] {
                    ui.label(RichText::new(head).font(theme::mono(10.5)).color(p.dim));
                }
                ui.end_row();
                for row in rows.iter().filter(|r| r.kind == *kind) {
                    result(ui, row, p);
                    ui.end_row();
                }
            });
    }
}

fn result(ui: &mut Ui, row: &Row, p: &Palette) {
    let mono = |text: String, colour| RichText::new(text).font(theme::mono(11.5)).color(colour);
    let name = ui.label(mono(row.candidate.clone(), p.text));
    if !row.providers.is_empty() {
        name.on_hover_text(format!("served by {}", row.providers.join(", ")));
    }
    let rate = row.rate();
    let colour = if rate >= 0.8 {
        p.ok
    } else if rate >= 0.5 {
        p.warn
    } else {
        p.bad
    };
    ui.label(mono(format!("{}/{}", row.passed, row.runs), colour));
    ui.label(mono(
        row.tests
            .map_or("·".to_owned(), |t| format!("{:.0}%", t * 100.0)),
        p.dim,
    ));
    ui.label(mono(
        row.score.map_or("·".to_owned(), |s| format!("{s:.1}")),
        p.text,
    ));
    let per_run = match (row.billed.0 > 0.0, row.subscription_tokens > 0) {
        (true, _) => row.billed.to_string(),
        (false, true) => format!("{} tok", TokenCount(row.subscription_tokens)),
        (false, false) => "·".to_owned(),
    };
    ui.label(mono(per_run, p.text));
    ui.label(mono(
        row.per_success
            .filter(|_| row.billed.0 > 0.0)
            .map_or("·".to_owned(), |c| c.to_string()),
        p.accent,
    ));
    ui.label(mono(minutes(row.seconds), p.dim));
}

fn minutes(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    }
}
