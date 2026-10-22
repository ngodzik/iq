//! The agents at work: who works on what, each one's thread, the questions
//! waiting for the person, and where the person writes. Draws the state and
//! turns clicks and keys into its methods, giving back what the person
//! asked of the crew.

use bevy_egui::egui::{
    self, Color32, CornerRadius, Frame, Key, KeyboardShortcut, Margin, Modifiers, RichText, Ui,
};
use iq_ui::crew::{AgentView, CrewView, Entry, Request};

use crate::theme::{
    ACCENT, CYAN, DIM, EDGE, GREEN, LINK, PANEL, RAISED, RED, SELECTED, TEXT, YELLOW,
};

/// The most of a tool's output shown under it.
const OUTPUT_LINES: usize = 12;

/// Draws the page in `root`, the window's area under its tabs. Returns
/// what the person asked of the crew.
pub(crate) fn show(root: &mut Ui, crew: &mut CrewView) -> Vec<Request> {
    let mut asked = Vec::new();
    egui::Panel::left("agents")
        .resizable(true)
        .default_size(240.0)
        .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(12)))
        .show(root, |ui| agent_list(ui, crew, &mut asked));
    egui::Panel::bottom("prompt")
        .frame(Frame::new().fill(PANEL).inner_margin(Margin::same(12)))
        .show(root, |ui| prompt(ui, crew, &mut asked));
    egui::CentralPanel::default().show(root, |ui| {
        questions(ui, crew, &mut asked);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| match crew.selected() {
                Some(agent) => thread(ui, agent),
                None => welcome(ui, crew),
            });
    });
    asked
}

fn agent_list(ui: &mut Ui, crew: &mut CrewView, asked: &mut Vec<Request>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Agents").strong().color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let on = crew.autopilot();
            let label = RichText::new(if on { "Autopilot on" } else { "Autopilot off" })
                .color(if on { GREEN } else { DIM });
            if ui
                .selectable_label(on, label)
                .on_hover_text(
                    "On, a reviewing model answers what stays within the sandbox; what widens \
                     it still waits for you.",
                )
                .clicked()
            {
                asked.push(crew.toggle_autopilot());
            }
        });
    });
    let open = crew.internet();
    let label = RichText::new(if open {
        "Internet open"
    } else {
        "Internet asked"
    })
    .color(if open { YELLOW } else { DIM });
    if ui
        .selectable_label(open, label)
        .on_hover_text(
            "Asked: commands reach localhost, and the internet only when they fetch packages or \
             you allow it. Open: every command reaches it; files stay held and secrets out.",
        )
        .clicked()
    {
        asked.push(crew.toggle_internet());
    }
    ui.add_space(6.0);
    let mut chosen = None;
    let shown = crew.selected().map(|agent| agent.name.clone());
    for (index, agent) in crew.agents().iter().enumerate() {
        let selected = shown.as_deref() == Some(agent.name.as_str());
        let response = Frame::new()
            .fill(if selected {
                SELECTED
            } else {
                Color32::TRANSPARENT
            })
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (dot, colour) = if agent.working {
                        ("●", GREEN)
                    } else {
                        ("○", DIM)
                    };
                    ui.label(RichText::new(dot).color(colour));
                    ui.label(RichText::new(&agent.name).strong().color(TEXT));
                });
                ui.label(RichText::new(&agent.model).small().monospace().color(DIM));
                ui.label(RichText::new(spent(agent)).small().color(DIM));
            })
            .response
            .interact(egui::Sense::click());
        if response.clicked() {
            chosen = Some(index);
        }
    }
    if let Some(index) = chosen {
        crew.select(index);
    }
    if crew.agents().is_empty() {
        ui.label(RichText::new("None yet.").color(DIM));
    }
}

/// What an agent's calls cost, as its card says.
fn spent(agent: &AgentView) -> String {
    match (agent.billed.0 > 0.0, agent.estimated.0 > 0.0) {
        (true, true) => format!("{} billed, ~{} subscription", agent.billed, agent.estimated),
        (true, false) => format!("{} billed", agent.billed),
        (false, true) => format!("~{} subscription", agent.estimated),
        (false, false) => "nothing spent".to_owned(),
    }
}

fn questions(ui: &mut Ui, crew: &mut CrewView, asked: &mut Vec<Request>) {
    let mut answered = Vec::new();
    for question in crew.questions() {
        Frame::new()
            .fill(RAISED)
            .stroke(egui::Stroke::new(
                1.0,
                if question.widens { RED } else { YELLOW },
            ))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(format!("{} asks to {}", question.agent, question.action))
                        .strong()
                        .color(TEXT),
                );
                let mut why = format!("Held because {}.", question.reason);
                if question.widens {
                    why.push_str(" It reaches beyond the sandbox: only you may allow it.");
                }
                ui.label(RichText::new(why).color(DIM));
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Allow").color(GREEN)).clicked() {
                        answered.push((question.id, true));
                    }
                    if ui.button(RichText::new("Refuse").color(RED)).clicked() {
                        answered.push((question.id, false));
                    }
                });
            });
        ui.add_space(6.0);
    }
    for (id, approved) in answered {
        asked.push(crew.answer(id, approved));
    }
}

fn welcome(ui: &mut Ui, crew: &CrewView) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("What should iq do?")
                .size(22.0)
                .strong()
                .color(TEXT),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "The lead runs on {}; it may start other agents. The reviewer, for the \
                 autopilot, on {}.",
                crew.lead(),
                crew.reviewer()
            ))
            .color(DIM),
        );
        ui.label(
            RichText::new(
                "Files are read and written through the gate; commands run in the sandbox, with \
                 no network unless you allow it.",
            )
            .color(DIM),
        );
    });
}

fn thread(ui: &mut Ui, agent: &AgentView) {
    for entry in &agent.thread {
        match entry {
            Entry::Prompt(text) => bubble(ui, "Given", text, LINK),
            Entry::Text(text) => {
                ui.add_space(4.0);
                ui.label(RichText::new(text).color(TEXT));
            }
            Entry::Tool {
                name,
                input,
                output,
                ok,
            } => {
                let colour = if *ok { CYAN } else { RED };
                egui::CollapsingHeader::new(
                    RichText::new(format!("{name}  {input}"))
                        .monospace()
                        .color(colour),
                )
                .id_salt(ui.next_auto_id())
                .default_open(!ok)
                .show(ui, |ui| {
                    let lines: Vec<&str> = output.lines().collect();
                    let shown = if lines.len() > OUTPUT_LINES {
                        format!(
                            "{}\n[{} more lines]",
                            lines[..OUTPUT_LINES].join("\n"),
                            lines.len() - OUTPUT_LINES
                        )
                    } else {
                        output.clone()
                    };
                    ui.label(RichText::new(shown).small().monospace().color(DIM));
                });
            }
            Entry::Reviewed {
                action,
                approved,
                why,
            } => {
                let (word, colour) = if *approved {
                    ("allowed", GREEN)
                } else {
                    ("refused", RED)
                };
                ui.label(
                    RichText::new(format!("The reviewer {word} {action}: {why}"))
                        .small()
                        .color(colour),
                );
            }
            Entry::Asked {
                action,
                reason,
                answer,
                ..
            } => {
                let (word, colour) = match answer {
                    None => ("waits for you", YELLOW),
                    Some(true) => ("you allowed it", GREEN),
                    Some(false) => ("refused", RED),
                };
                ui.label(
                    RichText::new(format!("Asked to {action} ({reason}): {word}"))
                        .small()
                        .color(colour),
                );
            }
            Entry::Message { from, to, text } => {
                bubble(ui, &format!("{from} to {to}"), text, ACCENT);
            }
            Entry::Notice(text) => {
                ui.label(RichText::new(text).small().italics().color(YELLOW));
            }
            Entry::Done(Ok(_)) => {
                ui.label(RichText::new("Done.").small().color(GREEN));
                ui.separator();
            }
            Entry::Done(Err(error)) => {
                ui.label(
                    RichText::new(format!("Stopped: {error}"))
                        .small()
                        .color(RED),
                );
                ui.separator();
            }
        }
    }
}

fn bubble(ui: &mut Ui, title: &str, text: &str, colour: Color32) {
    ui.add_space(6.0);
    Frame::new()
        .fill(RAISED)
        .stroke(egui::Stroke::new(1.0, EDGE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).small().strong().color(colour));
            ui.label(RichText::new(text).color(TEXT));
        });
    ui.add_space(4.0);
}

fn prompt(ui: &mut Ui, crew: &mut CrewView, asked: &mut Vec<Request>) {
    let to = crew
        .selected()
        .map(|agent| format!("To {}", agent.name))
        .unwrap_or_else(|| "A task for the lead".to_owned());
    let working = crew.selected().is_some_and(|agent| agent.working);
    ui.horizontal(|ui| {
        ui.label(RichText::new(to).small().color(DIM));
        ui.label(
            RichText::new("Enter: send · Shift-Enter: new line")
                .small()
                .color(DIM),
        );
        if working {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("Stop").color(RED)).clicked()
                    && let Some(stop) = crew.stop()
                {
                    asked.push(stop);
                }
            });
        }
    });
    let id = egui::Id::new("prompt-text");
    let focused = ui.memory(|memory| memory.has_focus(id));
    let enter = focused && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let edit = egui::TextEdit::multiline(crew.input_mut())
        .id(id)
        .desired_rows(3)
        .desired_width(f32::INFINITY)
        .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
        .hint_text("Write here")
        .show(ui);
    // Ready for the first task as the window opens.
    if crew.agents().is_empty() && !focused {
        edit.response.request_focus();
    }
    if enter && let Some(request) = crew.send() {
        asked.push(request);
    }
}
