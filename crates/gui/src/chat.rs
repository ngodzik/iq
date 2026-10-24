//! A chat in a pane: what its agent was given and said, drawn as the
//! Markdown it is, the tools it called, the messages it wrote and got, what
//! it asks the person, and where the person writes to it.

use std::time::Duration;

use bevy_egui::egui::{
    self, CornerRadius, Frame, Id, Key, KeyboardShortcut, Margin, Modifiers, RichText, Sense,
    Stroke, Ui, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use iq_ui::crew::{AgentView, Entry, Question};

use crate::theme::{self, MONO_SIZE, Palette};

/// The most lines of a tool's output shown under it.
const OUTPUT_LINES: usize = 14;

/// The longest input of a tool shown on its line, in characters.
const INPUT_CHARS: usize = 90;

/// The room the hint beside what the person writes takes.
const SEND_ROOM: f32 = 64.0;

/// How long the light takes to run along the line of an agent at work.
const SHIMMER_SECONDS: f64 = 2.4;

/// `text` with the folder iq works in written `.`, and the person's home
/// `~`: paths read shorter, and the rest of them is what tells them apart.
pub(crate) fn short(text: &str) -> String {
    static PLACES: std::sync::OnceLock<Vec<(String, &'static str)>> = std::sync::OnceLock::new();
    let places = PLACES.get_or_init(|| {
        let mut places = Vec::new();
        if let Ok(folder) = std::env::current_dir() {
            places.push((format!("{}/", folder.display()), "./"));
            places.push((folder.display().to_string(), "."));
        }
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            places.push((std::path::Path::new(&home).display().to_string(), "~"));
        }
        places
    });
    places.iter().fold(text.to_owned(), |text, (long, short)| {
        text.replace(long, short)
    })
}

/// What the person did in a pane.
#[derive(Debug, Default)]
pub(crate) struct Done {
    /// The questions answered: their number, and whether allowed.
    pub(crate) answered: Vec<(u64, bool)>,
    /// Whether they sent what they wrote.
    pub(crate) sent: bool,
}

/// An agent's thread, oldest first. `working` is how long its turn has
/// lasted, if it works; `stop` the key that stops it.
pub(crate) fn thread(
    ui: &mut Ui,
    agent: &AgentView,
    working: Option<Duration>,
    stop: &str,
    cache: &mut CommonMarkCache,
    p: &Palette,
) {
    ui.spacing_mut().item_spacing.y = 8.0;
    for (index, entry) in agent.thread.iter().enumerate() {
        match entry {
            Entry::Prompt(text) => you(ui, text, p),
            Entry::Text(text) if text.trim().is_empty() => {}
            Entry::Text(text) => {
                CommonMarkViewer::new().show(ui, cache, text);
            }
            Entry::Tool {
                name,
                input,
                output,
                ok,
            } => tool(ui, (&agent.name, index), name, input, output, *ok, p),
            Entry::Reviewed {
                action,
                approved,
                why,
            } => {
                let (word, colour) = if *approved {
                    ("allowed", p.ok)
                } else {
                    ("refused", p.bad)
                };
                ui.label(
                    RichText::new(format!("The reviewer {word} {}: {why}", short(action)))
                        .small()
                        .color(colour),
                );
            }
            // While it waits, the question itself shows below the thread.
            Entry::Asked { answer: None, .. } => {}
            Entry::Asked {
                action,
                answer: Some(approved),
                ..
            } => {
                let (word, colour) = if *approved {
                    ("You allowed", p.ok)
                } else {
                    ("Refused:", p.bad)
                };
                ui.label(
                    RichText::new(format!("{word} {}", short(action)))
                        .small()
                        .color(colour),
                );
            }
            Entry::Message { from, to, text } => {
                ui.label(
                    RichText::new(format!("✉ {from} → {to}"))
                        .small()
                        .color(p.dim),
                );
                Frame::new()
                    .fill(p.card)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        CommonMarkViewer::new().show(ui, cache, text);
                    });
            }
            Entry::Notice(text) => {
                ui.label(RichText::new(text).small().italics().color(p.warn));
            }
            Entry::Done(Ok(_)) => {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                theme::light_line(
                    ui.painter(),
                    rect.left(),
                    rect.right(),
                    rect.center().y,
                    theme::alpha(p.dim, 0.5),
                );
            }
            Entry::Done(Err(error)) => {
                ui.label(
                    RichText::new(format!("Stopped: {error}"))
                        .small()
                        .color(p.bad),
                );
            }
        }
    }
    if let Some(lasted) = working {
        let phase = (ui.input(|i| i.time) % SHIMMER_SECONDS / SHIMMER_SECONDS) as f32;
        let line = format!("Working… {}s · {stop} to stop", lasted.as_secs());
        ui.label(theme::shimmer(&line, phase, p.dim, p.accent));
    }
}

/// What the person gave: their task, or a message that woke the agent.
fn you(ui: &mut Ui, text: &str, p: &Palette) {
    Frame::new()
        .fill(p.you)
        .stroke(Stroke::new(1.0, p.edge))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(
                    RichText::new("›")
                        .font(theme::mono(MONO_SIZE + 1.0))
                        .color(p.accent),
                );
                ui.add(egui::Label::new(RichText::new(text).color(p.text)).wrap());
            });
        });
}

/// A tool's line: its name and input, whether it failed; a click shows
/// what it returned, which shows from the start when it failed.
fn tool(
    ui: &mut Ui,
    at: (&str, usize),
    name: &str,
    input: &str,
    output: &str,
    ok: bool,
    p: &Palette,
) {
    let id = Id::new(("tool", at.0, at.1));
    let open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(!ok);
    let line = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            // A selectable label takes the click meant for the line.
            ui.style_mut().interaction.selectable_labels = false;
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
            let colour = if ok { p.dim } else { p.bad };
            ui.painter()
                .circle_filled(dot.center(), 2.6, theme::alpha(colour, 0.9));
            ui.label(
                RichText::new(name)
                    .font(theme::mono_medium(MONO_SIZE))
                    .color(p.text),
            );
            let input = short(input);
            let cut: String = input.chars().take(INPUT_CHARS).collect();
            let more = if input.chars().count() > INPUT_CHARS {
                "…"
            } else {
                ""
            };
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{cut}{more}"))
                        .font(theme::mono(MONO_SIZE))
                        .color(p.dim),
                )
                .truncate(),
            );
            if !ok {
                ui.label(RichText::new("failed").small().color(p.bad));
            }
        })
        .response
        .interact(Sense::click());
    if line.clicked() {
        ui.data_mut(|d| d.insert_temp(id, !open));
    }
    if open && !output.trim().is_empty() {
        let lines: Vec<&str> = output.lines().collect();
        let mut shown = lines
            .iter()
            .take(OUTPUT_LINES)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        if lines.len() > OUTPUT_LINES {
            shown.push_str(&format!("\n… {} more lines", lines.len() - OUTPUT_LINES));
        }
        Frame::new()
            .fill(p.code)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(shown)
                        .font(theme::mono(MONO_SIZE - 1.0))
                        .color(p.dim),
                );
            });
    }
}

/// What the pane's agents ask the person, each with its two answers.
/// `keys` names the shortcuts that give them.
pub(crate) fn questions(
    ui: &mut Ui,
    asked: &[&Question],
    keys: (&str, &str),
    p: &Palette,
    done: &mut Done,
) {
    for question in asked {
        Frame::new()
            .fill(theme::alpha(p.warn, 0.08))
            .stroke(Stroke::new(1.0, theme::alpha(p.warn, 0.45)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(format!(
                        "{} asks to {}",
                        question.agent,
                        short(&question.action)
                    ))
                    .color(p.text),
                );
                let mut why = format!("Held because {}.", short(&question.reason));
                if question.widens {
                    why.push_str(" It reaches beyond the sandbox: only you may allow it.");
                }
                ui.label(RichText::new(why).small().color(p.dim));
                ui.horizontal(|ui| {
                    let allow = RichText::new(format!("Allow  {}", keys.0)).color(p.ok);
                    if ui.button(allow).clicked() {
                        done.answered.push((question.id, true));
                    }
                    let refuse = RichText::new(format!("Refuse  {}", keys.1)).color(p.text);
                    if ui.button(refuse).clicked() {
                        done.answered.push((question.id, false));
                    }
                });
            });
    }
}

/// Where the person writes, at the bottom of the pane at `pane`: Enter
/// sends, Shift-Enter starts a new line. It takes the keys while its pane
/// has the focus.
pub(crate) fn input(
    ui: &mut Ui,
    pane: usize,
    text: &mut String,
    hint: &str,
    focused: bool,
    p: &Palette,
    done: &mut Done,
) {
    let id = Id::new(("input", pane));
    let has_focus = ui.memory(|m| m.has_focus(id));
    let enter = has_focus && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(
            1.0,
            if focused {
                theme::alpha(p.accent, 0.35)
            } else {
                p.edge
            },
        ))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::multiline(text)
                    .id(id)
                    .frame(Frame::NONE)
                    .desired_rows(1)
                    .desired_width(ui.available_width() - SEND_ROOM)
                    .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                    .hint_text(RichText::new(hint).color(p.dim))
                    .show(ui);
                ui.label(RichText::new("⏎ send").font(theme::mono(11.0)).color(p.dim));
                // Typing goes to the pane with the focus, whatever was
                // clicked last.
                if focused && !has_focus && ui.memory(|m| m.focused().is_none()) {
                    edit.response.request_focus();
                }
            });
        });
    if enter {
        done.sent = true;
    }
}
