//! iq's window: draws the state of `iq-ui` on the GPU, with egui on Bevy.
//! Its panes sit over glass the desktop shows through; a status line runs
//! along the bottom, the menu and the costs show when asked.
//!
//! The window sleeps between events. The program wakes it when it sends
//! something, the crew's work as it happens, and it wakes on its own every
//! few seconds besides, so that it costs nothing while nothing changes.
//! While an agent works, it draws often enough for its light to move.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod chat;
mod costs_page;
mod panes;
mod steward_view;
mod subscriptions_page;
mod theme;

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::window::CompositeAlphaMode;
use bevy::winit::{
    EventLoopProxy, EventLoopProxyWrapper, UpdateMode, WinitSettings, WinitUserEvent,
};
use bevy_egui::{
    EguiContexts, EguiGlobalSettings, EguiInput, EguiInputSet, EguiPlugin, EguiPreUpdateSet,
    EguiPrimaryContextPass, PrimaryEguiContext, egui,
};
use egui_commonmark::CommonMarkCache;
use iq_ui::crew::Request;
use iq_ui::keys::{Action, Chord};
use iq_ui::look::Look;
use iq_ui::{Board, Update};

use crate::theme::Palette;

/// How long the window sleeps without an event while it has the focus,
/// before taking what the loop read.
const AWAKE_WAIT: Duration = Duration::from_secs(2);

/// The same without the focus.
const ASLEEP_WAIT: Duration = Duration::from_secs(10);

/// How long it sleeps while an agent works: its light moves at this pace.
const BUSY_WAIT: Duration = Duration::from_millis(40);

/// How the window's pixels blend with what is behind it. Metal offers only
/// `Opaque` and `PostMultiplied`: asked for another, the surface cannot be
/// made and the window stops at its first frame.
#[cfg(target_os = "macos")]
const COMPOSITE_ALPHA: CompositeAlphaMode = CompositeAlphaMode::PostMultiplied;
#[cfg(not(target_os = "macos"))]
const COMPOSITE_ALPHA: CompositeAlphaMode = CompositeAlphaMode::PreMultiplied;

/// The height of macOS's title bar, which a see-through window draws its
/// content under.
const TITLE_BAR: f32 = 28.0;

/// Wakes the window from another thread, once it is open.
#[derive(Clone, Default)]
pub struct Waker(Arc<Mutex<Option<EventLoopProxy<WinitUserEvent>>>>);

impl Waker {
    /// Wakes the window, to take what was sent to it; nothing before it
    /// opens, as it takes everything sent then when it does.
    pub fn wake(&self) {
        if let Some(proxy) = self.0.lock().ok().as_ref().and_then(|p| p.as_ref()) {
            let _ = proxy.send_event(WinitUserEvent::WakeUp);
        }
    }
}

/// The window's state, as Bevy holds it.
#[derive(Resource)]
struct State {
    board: Board,
    /// What the program sends, as it sends it.
    feed: Mutex<Receiver<Update>>,
    /// What the person asks of the crew, for the program to carry out.
    requests: Sender<Request>,
    waker: Waker,
    /// What the keys asked since the last frame.
    actions: Vec<Action>,
    /// What the Markdown viewer keeps between frames.
    cache: CommonMarkCache,
    /// Since when each agent at work has worked.
    since: HashMap<String, Instant>,
    /// The look egui's widgets were last given.
    shown: Option<Look>,
    /// Whether the window was made see-through.
    see_through: bool,
    /// How the panes move, and the messages in flight.
    motion: panes::Motion,
}

/// Opens the window and runs it until it is closed: `board` as the
/// settings made it, what `feed` brings, what the person asks of the crew
/// sent on `requests`. `waker` wakes it once it is open.
pub fn run(feed: Receiver<Update>, requests: Sender<Request>, waker: Waker, board: Board) {
    // A window is made see-through when it is made, or never: set opaque,
    // it is opaque for good, and costs the desktop no blending; where the
    // system cannot blend windows, that is also the way to open one.
    let see_through = board.look().opacity() < 1.0;
    App::new()
        // Behind the panes, nothing: what shows through is the desktop,
        // through the glass's own alpha.
        .insert_resource(ClearColor(Color::NONE))
        .insert_resource(WinitSettings {
            focused_mode: UpdateMode::reactive(AWAKE_WAIT),
            unfocused_mode: UpdateMode::reactive_low_power(ASLEEP_WAIT),
        })
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "iq".to_owned(),
                        resolution: (1360, 860).into(),
                        // Made see-through from the start: a window cannot
                        // become so later.
                        transparent: see_through,
                        composite_alpha_mode: if see_through {
                            COMPOSITE_ALPHA
                        } else {
                            CompositeAlphaMode::Auto
                        },
                        fullsize_content_view: see_through && cfg!(target_os = "macos"),
                        titlebar_transparent: see_through && cfg!(target_os = "macos"),
                        ..default()
                    }),
                    ..default()
                })
                // The program sets up its own logging, which Bevy's would
                // find taken.
                .disable::<bevy::log::LogPlugin>(),
        )
        .add_plugins(EguiPlugin::default())
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        .insert_resource(State {
            board,
            feed: Mutex::new(feed),
            requests,
            waker,
            actions: Vec::new(),
            cache: CommonMarkCache::default(),
            since: HashMap::new(),
            shown: None,
            see_through,
            motion: panes::Motion::default(),
        })
        .add_systems(Startup, (camera, wakeable))
        .add_systems(
            PreUpdate,
            keys.after(EguiInputSet::WriteEguiEvents)
                .before(EguiPreUpdateSet::BeginPass),
        )
        .add_systems(Update, (take_feed, pace).chain())
        .add_systems(EguiPrimaryContextPass, draw)
        .run();
}

/// The camera egui draws with.
fn camera(mut commands: Commands) {
    commands.spawn((Camera2d, PrimaryEguiContext));
}

/// Lets the program wake the window.
fn wakeable(state: Res<State>, proxy: Res<EventLoopProxyWrapper>) {
    if let Ok(mut slot) = state.waker.0.lock() {
        *slot = Some((**proxy).clone());
    }
}

/// Takes the keys the shortcuts bind from what egui is about to read, for
/// their actions rather than for the widgets. A key held down acts once.
fn keys(mut inputs: Query<&mut EguiInput>, mut state: ResMut<State>) {
    let State { board, actions, .. } = &mut *state;
    let keymap = board.keys();
    for mut input in &mut inputs {
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let chord = Chord {
                ctrl: modifiers.ctrl,
                alt: modifiers.alt,
                shift: modifiers.shift,
                cmd: modifiers.mac_cmd,
                key: key.name().to_owned(),
            };
            match keymap.action(&chord) {
                Some(action) => {
                    if *pressed && !*repeat {
                        actions.push(action);
                    }
                    false
                }
                None => true,
            }
        });
    }
}

/// Takes what the program sent since the last frame, and notes when each
/// agent started working.
fn take_feed(mut state: ResMut<State>) {
    let updates: Vec<Update> = state
        .feed
        .lock()
        .map(|feed| feed.try_iter().collect())
        .unwrap_or_default();
    if updates.is_empty() {
        return;
    }
    for update in updates {
        state.board.update(update);
    }
    let State { board, since, .. } = &mut *state;
    for agent in board.crew().agents() {
        if agent.working {
            since.entry(agent.name.clone()).or_insert_with(Instant::now);
        } else {
            since.remove(&agent.name);
        }
    }
}

/// Draws often while an agent works or a chat starts, for their light to
/// move; sleeps otherwise.
fn pace(state: Res<State>, mut settings: ResMut<WinitSettings>, mut busy: Local<bool>) {
    let now = !state.since.is_empty()
        || !state.board.layout().starting().is_empty()
        || state.motion.moving();
    if now == *busy {
        return;
    }
    *busy = now;
    *settings = if now {
        WinitSettings {
            focused_mode: UpdateMode::reactive(BUSY_WAIT),
            unfocused_mode: UpdateMode::reactive_low_power(BUSY_WAIT * 3),
        }
    } else {
        WinitSettings {
            focused_mode: UpdateMode::reactive(AWAKE_WAIT),
            unfocused_mode: UpdateMode::reactive_low_power(ASLEEP_WAIT),
        }
    };
}

/// Draws the state, and carries out what the person did: the keys' and
/// the clicks' actions on the window, what they ask of the crew sent to
/// the program.
fn draw(
    mut contexts: EguiContexts,
    mut state: ResMut<State>,
    mut exit: MessageWriter<AppExit>,
    mut styled: Local<bool>,
    mut warmed: Local<bool>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    if !*styled {
        theme::fonts(&ctx);
        *styled = true;
        // The fonts set take effect on the next pass.
        ctx.request_repaint();
        return Ok(());
    }
    if !*warmed {
        theme::warm(&ctx);
        *warmed = true;
    }
    let state = &mut *state;
    if state.board.quitting() {
        exit.write(AppExit::Success);
    }
    let mut actions = std::mem::take(&mut state.actions);
    // The steward looks every few seconds; what it finds may ask the crew
    // to close a payer, or have it write to an agent.
    let mut asked = state.board.watch(iq_ui::now());
    act(state, &mut actions, &mut asked, &mut exit);
    let look = state.board.look();
    if state.shown != Some(look) {
        theme::visuals(&ctx, &theme::palette(look));
        state.shown = Some(look);
    }
    let p = theme::palette(look);
    let window = ctx.content_rect();
    let mut root = egui::Ui::new(
        ctx.clone(),
        egui::Id::new("iq"),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(window),
    );
    root.painter().rect_filled(window, 0.0, p.ground);
    let bare = egui::Frame::NONE;
    if cfg!(target_os = "macos") && state.see_through {
        egui::Panel::top("title-bar")
            .exact_size(TITLE_BAR)
            .frame(bare)
            .show(&mut root, |_| {});
    }
    if state.board.menu() {
        egui::Panel::top("menu")
            .frame(bare.inner_margin(egui::Margin::symmetric(16, 8)))
            .show(&mut root, |ui| menu(ui, &state.board, &p, &mut actions));
    }
    let steward_at = egui::Panel::bottom("status")
        .frame(bare.inner_margin(egui::Margin::symmetric(18, 7)))
        .show(&mut root, |ui| status(ui, &state.board, &p, &mut actions))
        .inner;
    state.motion.set_steward_at(steward_at);
    if state.board.costs_shown() {
        egui::Panel::right("costs")
            .resizable(true)
            .default_size(400.0)
            .min_size(300.0)
            .frame(
                bare.fill(p.pane)
                    .stroke(egui::Stroke::new(1.0, p.edge))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(14))
                    .outer_margin(egui::Margin {
                        left: 0,
                        right: 12,
                        top: 12,
                        bottom: 0,
                    }),
            )
            .show(&mut root, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("costs-side")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        costs_page::show(ui, &mut state.board, true, &p);
                        ui.add_space(14.0);
                        subscriptions_page::show(ui, state.board.subscriptions(), &p);
                    });
            });
    }
    egui::CentralPanel::default()
        .frame(bare.inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: 12,
            bottom: 0,
        }))
        .show(&mut root, |ui| {
            let mut paint = panes::Paint {
                p: &p,
                cache: &mut state.cache,
                since: &state.since,
                motion: &mut state.motion,
            };
            asked.extend(panes::show(ui, &mut state.board, &mut paint));
        });
    if state.board.keys_shown() {
        shortcuts(&ctx, &state.board, &p, &mut actions);
    }
    // What the clicks asked takes effect on the next frame, as the keys'.
    state.actions.extend(actions);
    if !state.actions.is_empty() {
        ctx.request_repaint();
    }
    for request in asked {
        // The program is gone only as the window closes.
        let _ = state.requests.send(request);
    }
    Ok(())
}

/// Carries out `actions`: quitting here, the rest on the board, and what
/// they ask of the crew into `asked`.
fn act(
    state: &mut State,
    actions: &mut Vec<Action>,
    asked: &mut Vec<Request>,
    exit: &mut MessageWriter<AppExit>,
) {
    for action in actions.drain(..) {
        if action == Action::Quit {
            exit.write(AppExit::Success);
        } else if let Some(request) = state.board.act(action) {
            asked.push(request);
        }
    }
}

/// The key that does `action`, as a hint writes it.
fn chord(board: &Board, action: Action) -> String {
    board
        .keys()
        .chord(action)
        .map(ToString::to_string)
        .unwrap_or_default()
}

/// The status line: the autopilot, the internet and the steward, which a
/// click turns, the agents at work, and the keys most needed. Returns where
/// the steward is drawn, for its letters to fly from.
fn status(
    ui: &mut egui::Ui,
    board: &Board,
    p: &Palette,
    actions: &mut Vec<Action>,
) -> Option<egui::Pos2> {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        let crew = board.crew();
        let small = |text: String, colour| egui::RichText::new(text).small().color(colour);
        let autopilot = if crew.autopilot() {
            small("⏵⏵ Autopilot".to_owned(), p.accent)
        } else {
            small("Autopilot off".to_owned(), p.dim)
        };
        if ui
            .add(egui::Label::new(autopilot).sense(egui::Sense::click()))
            .on_hover_text(format!(
                "{}: a reviewing model answers what stays within the sandbox, and what only \
                 reaches the internet; publishing and files outside the workspace still wait for \
                 you.",
                chord(board, Action::Autopilot)
            ))
            .clicked()
        {
            actions.push(Action::Autopilot);
        }
        let internet = if crew.internet() {
            small("Internet: open".to_owned(), p.warn)
        } else {
            small("Internet: packages".to_owned(), p.dim)
        };
        if ui
            .add(egui::Label::new(internet).sense(egui::Sense::click()))
            .on_hover_text(
                "Packages: commands reach localhost, and the internet when they fetch packages or \
                 you allow it. Open: every command reaches it; files stay held, secrets out.",
            )
            .clicked()
        {
            actions.push(Action::Internet);
        }
        let steward = steward_mark(ui, board, p);
        if steward.clicked() {
            actions.push(Action::Steward);
        }
        let steward_at = steward.rect.left_center() + egui::vec2(6.0, 0.0);
        let working = crew.agents().iter().filter(|a| a.working).count();
        let waiting = crew.questions().len();
        let count = crew.agents().len();
        let mut said = format!("{count} agent{}", if count == 1 { "" } else { "s" });
        if working > 0 {
            said.push_str(&format!(" · {working} working"));
        }
        ui.label(small(said, p.dim));
        if waiting > 0 {
            ui.label(small(format!("{waiting} waiting for you"), p.warn));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (action, what) in [
                (Action::Keys, "keys"),
                (Action::Costs, "costs"),
                (Action::Split, "split"),
            ] {
                let key = chord(board, action);
                if !key.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("{key} {what}"))
                            .font(theme::mono(11.0))
                            .color(p.dim),
                    );
                }
            }
        });
        Some(steward_at)
    })
    .inner
}

/// The steward in the status line: a sphere coloured by how the worst
/// limit stands, pulsing when one is near, and its state.
fn steward_mark(ui: &mut egui::Ui, board: &Board, p: &Palette) -> egui::Response {
    use iq_ui::steward::Level;
    let worst = board
        .gauges()
        .iter()
        .map(|g| g.level)
        .max()
        .unwrap_or(Level::Calm);
    let (tint, said) = match worst {
        Level::Calm => (p.ok, "calm"),
        Level::Watch => (p.warn, "watching"),
        Level::Alert => (p.bad, "a limit is near"),
        Level::Reached => (p.bad, "a limit is reached"),
    };
    let on = board.steward().on();
    let text = if on {
        format!("Steward: {said}")
    } else {
        format!("Steward off · {said}")
    };
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            let time = ui.input(|i| i.time) as f32;
            let glow = match worst {
                Level::Calm => 0.0,
                Level::Watch => 0.4,
                Level::Alert | Level::Reached => 0.6 + 0.4 * (time * 3.0).sin(),
            };
            theme::sphere(ui.painter(), dot.center(), 3.8, tint, glow);
            ui.label(
                egui::RichText::new(text)
                    .small()
                    .color(if on { p.text } else { p.dim }),
            );
        })
        .response
        .interact(egui::Sense::click());
    response.on_hover_text(format!(
        "{}: the steward holds iq to your limits, and writes to the agent that spends too fast",
        chord(board, Action::Steward)
    ))
}

/// The menu: the window's actions, each with its key.
fn menu(ui: &mut egui::Ui, board: &Board, p: &Palette, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("iq")
                .font(theme::bold(15.0))
                .color(p.accent),
        );
        ui.add_space(10.0);
        let look = board.look();
        let entries = [
            (Action::Split, "Split".to_owned()),
            (Action::Zoom, "Zoom".to_owned()),
            (Action::NewChat, "New chat".to_owned()),
            (Action::Costs, "Costs".to_owned()),
            (Action::Theme, format!("Theme: {}", look.theme())),
            (
                Action::Opacity,
                format!("Opacity {:.0}%", look.opacity() * 100.0),
            ),
            (Action::Keys, "Shortcuts".to_owned()),
        ];
        for (action, name) in entries {
            if ui
                .button(egui::RichText::new(name).small())
                .on_hover_text(chord(board, action))
                .clicked()
            {
                actions.push(action);
            }
        }
    });
}

/// The list of shortcuts, over the panes.
fn shortcuts(ctx: &egui::Context, board: &Board, p: &Palette, actions: &mut Vec<Action>) {
    let mut open = true;
    egui::Window::new("Shortcuts")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("shortcuts")
                .num_columns(2)
                .spacing([22.0, 6.0])
                .show(ui, |ui| {
                    for (key, action) in board.keys().list() {
                        ui.label(
                            egui::RichText::new(key)
                                .font(theme::mono(12.0))
                                .color(p.accent),
                        );
                        ui.label(egui::RichText::new(action.what()).color(p.text));
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "Change them in the [keys] of ~/.iq/settings.toml, such as costs = \"ctrl+u\". \
                     Drag the lines between panes to share the window; double-click a title to \
                     have its pane over all.",
                )
                .small()
                .color(p.dim),
            );
        });
    if !open {
        actions.push(Action::Keys);
    }
}
