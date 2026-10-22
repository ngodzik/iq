//! iq's window: draws the state of `iq-ui` on the GPU, with egui on Bevy.
//!
//! The window sleeps between events. The program wakes it when it sends
//! something, the crew's work as it happens, and it wakes on its own every
//! few seconds besides, so that it costs nothing while nothing changes.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod agents_page;
mod costs_page;
mod subscriptions_page;
mod theme;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::app::AppExit;
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use bevy::winit::{
    EventLoopProxy, EventLoopProxyWrapper, UpdateMode, WinitSettings, WinitUserEvent,
};
use bevy_egui::{
    EguiContexts, EguiGlobalSettings, EguiInput, EguiInputSet, EguiPlugin, EguiPreUpdateSet,
    EguiPrimaryContextPass, PrimaryEguiContext, egui,
};
use iq_ui::crew::Request;
use iq_ui::{Board, Page, Update};

/// How long the window sleeps without an event while it has the focus,
/// before taking what the loop read.
const AWAKE_WAIT: Duration = Duration::from_secs(2);

/// The same without the focus.
const ASLEEP_WAIT: Duration = Duration::from_secs(10);

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
}

/// Opens the window and runs it until it is closed: the crew, the costs and
/// the subscriptions, as `feed` brings them, what the person asks of the
/// crew sent on `requests`. `waker` wakes it once it is open.
pub fn run(feed: Receiver<Update>, requests: Sender<Request>, waker: Waker) {
    App::new()
        .insert_resource(ClearColor(theme::BACKGROUND_COLOUR))
        .insert_resource(WinitSettings {
            focused_mode: UpdateMode::reactive(AWAKE_WAIT),
            unfocused_mode: UpdateMode::reactive_low_power(ASLEEP_WAIT),
        })
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "iq".to_owned(),
                        resolution: (1200, 800).into(),
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
            board: Board::default(),
            feed: Mutex::new(feed),
            requests,
            waker,
        })
        .add_systems(Startup, (camera, wakeable))
        .add_systems(
            PreUpdate,
            tab.after(EguiInputSet::WriteEguiEvents)
                .before(EguiPreUpdateSet::BeginPass),
        )
        .add_systems(Update, (quit, take_feed))
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

/// Takes Tab from what egui is about to read, for the next page rather
/// than for moving egui's focus.
fn tab(mut inputs: Query<&mut EguiInput>, mut state: ResMut<State>) {
    for mut input in &mut inputs {
        input.events.retain(|event| match event {
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed,
                modifiers,
                ..
            } if modifiers.is_none() => {
                if *pressed {
                    state.board.next_page();
                }
                false
            }
            _ => true,
        });
    }
}

/// Quits on Ctrl-Q, Cmd-Q on macOS. The modifier is followed through the
/// keys in their order rather than read once a frame: Ctrl and Q pressed
/// within one frame would otherwise be missed.
fn quit(
    mut pressed: MessageReader<KeyboardInput>,
    mut held: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let modifier = if cfg!(target_os = "macos") {
        [KeyCode::SuperLeft, KeyCode::SuperRight]
    } else {
        [KeyCode::ControlLeft, KeyCode::ControlRight]
    };
    for key in pressed.read() {
        let down = key.state == ButtonState::Pressed;
        if modifier.contains(&key.key_code) {
            *held = down;
        } else if down && *held && key.key_code == KeyCode::KeyQ {
            exit.write(AppExit::Success);
        }
    }
}

/// Takes what the program sent since the last frame.
fn take_feed(mut state: ResMut<State>) {
    let updates: Vec<Update> = state
        .feed
        .lock()
        .map(|feed| feed.try_iter().collect())
        .unwrap_or_default();
    for update in updates {
        state.board.update(update);
    }
}

/// Draws the state, and turns the person's clicks into changes of it: the
/// page, the period, and what they ask of the crew, sent to the program.
fn draw(mut contexts: EguiContexts, mut state: ResMut<State>, mut styled: Local<bool>) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    if !*styled {
        theme::apply(&ctx);
        *styled = true;
        // The fonts set take effect on the next pass.
        ctx.request_repaint();
        return Ok(());
    }
    // Panels are laid in a root of their own, the whole window.
    let mut root = egui::Ui::new(
        ctx.clone(),
        egui::Id::new("iq"),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.content_rect()),
    );
    egui::Panel::top("tabs")
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(16, 10)),
        )
        .show(&mut root, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("iq")
                        .font(theme::bold(18.0))
                        .color(theme::ACCENT),
                );
                ui.add_space(12.0);
                for page in Page::ALL {
                    if ui
                        .selectable_label(state.board.page() == page, page.name())
                        .clicked()
                    {
                        state.board.show(page);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Tab: next page · Ctrl-Q: quit")
                            .small()
                            .color(theme::DIM),
                    );
                    let waiting = state.board.crew().questions().len();
                    if waiting > 0 && state.board.page() != Page::Agents {
                        ui.label(
                            egui::RichText::new(format!("{waiting} waiting for you"))
                                .strong()
                                .color(theme::YELLOW),
                        );
                    }
                });
            });
        });
    if state.board.page() == Page::Agents {
        let asked = agents_page::show(&mut root, state.board.crew_mut());
        for request in asked {
            // The program is gone only as the window closes.
            let _ = state.requests.send(request);
        }
        return Ok(());
    }
    egui::CentralPanel::default().show(&mut root, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| match state.board.page() {
            Page::Costs => {
                if let Some(period) = costs_page::show(ui, &state.board) {
                    state.board.sum_over(period);
                }
            }
            Page::Subscriptions => subscriptions_page::show(ui, state.board.subscriptions()),
            Page::Agents => {}
        });
    });
    Ok(())
}
