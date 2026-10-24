//! The panes: laid out as the split says, each a lit card over the glass,
//! the one with the focus raised a step with a line of light along its top;
//! the lines between them dragged to share the window, a title clicked to
//! choose what a pane shows, double-clicked to have it over the whole
//! window.
//!
//! The panes move rather than jump: a pane opening grows from the right
//! edge of its place while the others give it room, one closing shrinks
//! away. A message between agents shown flies from the writer's pane to the
//! reader's, a spark with its first words, and lights the reader's edge as
//! it lands.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use bevy_egui::egui::{
    self, Align, CornerRadius, CursorIcon, Id, Layout, Rect, RichText, Sense, Shadow, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use egui_commonmark::CommonMarkCache;
use iq_ui::Board;
use iq_ui::crew::{AgentView, Request};
use iq_ui::keys::Action;
use iq_ui::layout::{NewChat, Shows, Split};

use crate::chat::{self, Done};
use crate::theme::{self, MONO_SIZE, Palette};
use crate::{costs_page, subscriptions_page};

/// The room between panes, where their lines are dragged.
const GAP: f32 = 10.0;

/// A pane's title bar.
const HEAD: f32 = 38.0;

/// The roundness of a pane.
const ROUND: u8 = 12;

/// How fast a pane goes to its place: the share of the way left it covers
/// in a second is `1 - e^-SPEED`, most of it in a tenth of a second.
const SPEED: f32 = 28.0;

/// How long a message takes to fly from one pane to another, in seconds.
const FLIGHT: f64 = 1.2;

/// How long a pane's edge stays lit once a message lands, or once it
/// opens, in seconds.
const LIT: f64 = 0.9;

/// How much of a message flies with it.
const FLIGHT_CHARS: usize = 38;

/// How the panes move between layouts, and the messages in flight.
#[derive(Debug, Default)]
pub(crate) struct Motion {
    /// Each pane's place as drawn, by what it shows.
    places: HashMap<String, Rect>,
    /// The panes closing: what they showed, and where they are.
    leaving: Vec<(String, Rect)>,
    /// The messages flying: from which pane, to which, what, since when.
    flights: Vec<(String, String, String, f64)>,
    /// When each pane last had its edge lit.
    lit: HashMap<String, f64>,
    /// Whether anything moved in the last frame.
    moving: bool,
    /// Whether a line between panes was dragged in the last frame: the
    /// panes then keep up with it, rather than chase it.
    dragging: bool,
    /// Where the steward is drawn, in the status line, for its letters to
    /// fly from and to.
    steward_at: Option<egui::Pos2>,
}

/// The name the steward's place is kept under.
const STEWARD_KEY: &str = "steward";

impl Motion {
    /// Where the steward is drawn this frame.
    pub(crate) fn set_steward_at(&mut self, at: Option<egui::Pos2>) {
        self.steward_at = at;
    }

    /// Whether anything moves, for the window to keep drawing.
    pub(crate) fn moving(&self) -> bool {
        self.moving
    }
}

/// What a pane shows, as a name its place is kept under while it moves.
fn key(board: &Board, index: usize) -> String {
    match board.layout().pane(index).map(|pane| &pane.shows) {
        Some(Shows::Chat(agent)) => format!("chat:{agent}"),
        Some(Shows::New(_)) => format!("new:{index}"),
        Some(Shows::Costs) => format!("costs:{index}"),
        Some(Shows::Subscriptions) => format!("subscriptions:{index}"),
        None => format!("none:{index}"),
    }
}

/// `from` moved toward `to` by `share` of the way.
fn toward(from: Rect, to: Rect, share: f32) -> Rect {
    Rect::from_min_max(
        from.min + (to.min - from.min) * share,
        from.max + (to.max - from.max) * share,
    )
}

/// `rect` closed to nothing at its right edge, as a pane opens from and
/// closes to.
fn folded(rect: Rect) -> Rect {
    Rect::from_min_max(pos2(rect.right(), rect.top()), rect.max)
}

/// How far apart two places are, at their farthest corner.
fn apart(a: Rect, b: Rect) -> f32 {
    (a.min - b.min).length().max((a.max - b.max).length())
}

/// What drawing the panes needs besides the board.
pub(crate) struct Paint<'a> {
    /// The theme's colours.
    pub(crate) p: &'a Palette,
    /// What the Markdown viewer keeps between frames.
    pub(crate) cache: &'a mut CommonMarkCache,
    /// Since when each agent at work has worked.
    pub(crate) since: &'a HashMap<String, Instant>,
    /// How the panes move.
    pub(crate) motion: &'a mut Motion,
}

/// Draws the panes in `ui`'s whole room, each where its motion has it.
/// Returns what the person asked of the crew.
pub(crate) fn show(ui: &mut Ui, board: &mut Board, paint: &mut Paint<'_>) -> Vec<Request> {
    let area = ui.max_rect();
    let now = ui.input(|i| i.time);
    let share = 1.0 - (-ui.input(|i| i.stable_dt).min(0.1) * SPEED).exp();
    let targets: Vec<(usize, String, Rect)> = rects(board, area)
        .into_iter()
        .map(|(index, rect)| (index, key(board, index), rect))
        .collect();
    let motion = &mut *paint.motion;
    let mut moving = false;
    let snap = motion.dragging;
    let present: HashSet<&str> = targets.iter().map(|(_, k, _)| k.as_str()).collect();
    let gone: Vec<String> = motion
        .places
        .keys()
        .filter(|k| !present.contains(k.as_str()))
        .cloned()
        .collect();
    for key in gone {
        if let Some(place) = motion.places.remove(&key) {
            motion.leaving.push((key, place));
        }
    }
    let mut rects = Vec::new();
    for (index, key, target) in &targets {
        let place = match motion.places.get(key) {
            Some(_) if snap => *target,
            Some(place) => toward(*place, *target, share),
            None => {
                // Opening: from nothing at the right edge of its place, lit.
                if !motion.places.is_empty() {
                    motion.lit.insert(key.clone(), now);
                }
                if motion.places.is_empty() {
                    *target
                } else {
                    folded(*target)
                }
            }
        };
        moving |= apart(place, *target) > 0.5;
        let place = if apart(place, *target) <= 0.5 {
            *target
        } else {
            place
        };
        motion.places.insert(key.clone(), place);
        rects.push((*index, key.clone(), place));
    }
    for (_, place) in &mut motion.leaving {
        *place = toward(*place, folded(*place), share);
    }
    motion.leaving.retain(|(_, place)| place.width() > 3.0);
    moving |= !motion.leaving.is_empty();
    // The lines are there once the panes stand, whatever flies or glows.
    let placing = moving;
    for post in board.take_posts() {
        let writer = if post.from_steward() {
            STEWARD_KEY.to_owned()
        } else {
            format!("chat:{}", post.from)
        };
        let readers: Vec<String> = if post.to_steward() {
            vec![STEWARD_KEY.to_owned()]
        } else if post.to_everyone() {
            targets
                .iter()
                .map(|(_, k, _)| k.clone())
                .filter(|k| k.starts_with("chat:") && *k != format!("chat:{}", post.from))
                .collect()
        } else if post.to_person() {
            // To the person: to their chat, the first pane.
            targets
                .first()
                .map(|(_, k, _)| k.clone())
                .into_iter()
                .collect()
        } else {
            vec![format!("chat:{}", post.to)]
        };
        for reader in readers {
            motion
                .flights
                .push((writer.clone(), reader, post.text.clone(), now));
        }
    }
    let p = *paint.p;
    for (key, place) in &motion.leaving {
        closing(ui, key, *place, &p);
    }
    // A press in a pane gives it the focus, before anything in it reacts.
    let pressed = ui.input(|i| {
        i.pointer
            .any_pressed()
            .then(|| i.pointer.interact_pos())
            .flatten()
    });
    if let Some(at) = pressed
        && let Some((index, _, _)) = rects.iter().find(|(_, _, rect)| rect.contains(at))
    {
        board.layout_mut().set_focus(*index);
    }
    let mut asked = Vec::new();
    for (index, key, rect) in &rects {
        let lit = paint
            .motion
            .lit
            .get(key)
            .map_or(0.0, |at| (1.0 - (now - at) / LIT).max(0.0) as f32);
        moving |= lit > 0.0;
        pane(ui, board, *index, *rect, lit, paint, &mut asked);
    }
    moving |= flights(ui, &rects, paint.motion, now, &p);
    paint.motion.dragging = !placing && lines(ui, board, area);
    paint.motion.moving = moving;
    if moving {
        ui.ctx().request_repaint();
    }
    asked
}

/// A pane closing: its card and its name, shrinking away.
fn closing(ui: &Ui, key: &str, place: Rect, p: &Palette) {
    let painter = ui.painter();
    let round = CornerRadius::same(ROUND);
    painter.rect_filled(place, round, p.pane);
    painter.rect_stroke(place, round, Stroke::new(1.0, p.edge), StrokeKind::Inside);
    let name = key.split_once(':').map_or(key, |(_, name)| name);
    if place.width() > 60.0 {
        painter.with_clip_rect(place).text(
            place.min + vec2(14.0, HEAD / 2.0),
            egui::Align2::LEFT_CENTER,
            name,
            theme::bold(13.0),
            theme::alpha(p.text, (place.width() / 300.0).min(1.0)),
        );
    }
}

/// The messages in flight, each a spark with a trail and its first words
/// on a curve from its writer's title to its reader's; once landed, the
/// reader's edge lights. Returns whether any still flies.
fn flights(
    ui: &Ui,
    rects: &[(usize, String, Rect)],
    motion: &mut Motion,
    now: f64,
    p: &Palette,
) -> bool {
    let steward_at = motion.steward_at;
    let at = |key: &str| {
        if key == STEWARD_KEY {
            return steward_at;
        }
        rects
            .iter()
            .find(|(_, k, _)| k == key)
            .map(|(_, _, rect)| rect.min + vec2(20.0, HEAD / 2.0))
    };
    let painter = ui.painter();
    let middle = ui.max_rect().center();
    let mut landed = Vec::new();
    motion.flights.retain(|(from, to, text, start)| {
        let (Some(a), Some(b)) = (at(from), at(to)) else {
            return false;
        };
        // The steward's letters fly in a colour of their own.
        let tint = if from == STEWARD_KEY || to == STEWARD_KEY {
            p.info
        } else {
            p.accent
        };
        let t = ((now - start) / FLIGHT).clamp(0.0, 1.0) as f32;
        if t >= 1.0 {
            landed.push(to.clone());
            return false;
        }
        // Eased in and out, along an arc that bends toward the middle of
        // the panes, never out of the window.
        let eased = t * t * (3.0 - 2.0 * t);
        let way = b - a;
        let halfway = a + way * 0.5;
        let bend = vec2(-way.y, way.x).normalized() * way.length().min(400.0) * 0.28;
        let control = if (halfway + bend - middle).length() <= (halfway - bend - middle).length() {
            halfway + bend
        } else {
            halfway - bend
        };
        let point = |s: f32| {
            let s = s.clamp(0.0, 1.0);
            let ab = a + (control - a) * s;
            let cb = control + (b - control) * s;
            ab + (cb - ab) * s
        };
        for step in (1..=12).rev() {
            let back = step as f32 * 0.022;
            let fade = 1.0 - step as f32 / 13.0;
            painter.circle_filled(
                point(eased - back),
                3.6 * fade,
                theme::alpha(tint, 0.45 * fade),
            );
        }
        let head = point(eased);
        theme::sphere(painter, head, 4.2, tint, 1.0);
        let words: String = text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(FLIGHT_CHARS)
            .collect();
        let more = if text.chars().count() > words.chars().count() {
            "…"
        } else {
            ""
        };
        let shown = (t * 6.0).min((1.0 - t) * 6.0).min(1.0);
        let galley = painter.layout_no_wrap(
            format!("✉ {words}{more}"),
            theme::mono(11.0),
            theme::alpha(p.text, shown),
        );
        let chip = Rect::from_min_size(head + vec2(12.0, -26.0), galley.size() + vec2(16.0, 8.0));
        painter.rect_filled(chip, 7.0, theme::alpha(theme::opaque(p.pane_focus), shown));
        painter.rect_stroke(
            chip,
            7.0,
            Stroke::new(1.0, theme::alpha(tint, 0.45 * shown)),
            StrokeKind::Inside,
        );
        painter.galley(chip.min + vec2(8.0, 4.0), galley, p.text);
        true
    });
    for key in landed {
        motion.lit.insert(key, now);
    }
    !motion.flights.is_empty()
}

/// Where each pane shown goes in `area`.
fn rects(board: &Board, area: Rect) -> Vec<(usize, Rect)> {
    let layout = board.layout();
    let shown = layout.shown();
    let (column, row) = layout.lines();
    let split_x = area.left() + area.width() * column;
    let split_y = area.top() + area.height() * row;
    let half = GAP / 2.0;
    let left = Rect::from_min_max(area.min, pos2(split_x - half, area.max.y));
    let right = Rect::from_min_max(pos2(split_x + half, area.min.y), area.max);
    let quarter = |x: (f32, f32), y: (f32, f32)| Rect::from_min_max(pos2(x.0, y.0), pos2(x.1, y.1));
    let xs = [
        (area.left(), split_x - half),
        (split_x + half, area.right()),
    ];
    let ys = [
        (area.top(), split_y - half),
        (split_y + half, area.bottom()),
    ];
    match (layout.zoomed(), layout.split()) {
        (true, _) | (_, Split::One) => shown.into_iter().map(|at| (at, area)).collect(),
        (false, Split::Two) => shown.into_iter().zip([left, right]).collect(),
        (false, Split::Three) => shown
            .into_iter()
            .zip([left, quarter(xs[1], ys[0]), quarter(xs[1], ys[1])])
            .collect(),
        (false, Split::Four) => shown
            .into_iter()
            .zip([
                quarter(xs[0], ys[0]),
                quarter(xs[1], ys[0]),
                quarter(xs[0], ys[1]),
                quarter(xs[1], ys[1]),
            ])
            .collect(),
    }
}

/// The lines between panes, dragged with the mouse or the touchpad; where
/// four meet, a grip moves both. Returns whether one is dragged.
fn lines(ui: &mut Ui, board: &mut Board, area: Rect) -> bool {
    let layout = board.layout();
    if layout.zoomed() || layout.split() == Split::One {
        return false;
    }
    let mut dragged = false;
    let four = layout.split() == Split::Four;
    let three = layout.split() == Split::Three;
    let (column, row) = layout.lines();
    let x = area.left() + area.width() * column;
    let y = area.top() + area.height() * row;
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let share_x = |at: egui::Pos2| (at.x - area.left()) / area.width();
    let share_y = |at: egui::Pos2| (at.y - area.top()) / area.height();
    let p_dim = ui.visuals().widgets.noninteractive.fg_stroke.color;
    if four {
        let grip = Rect::from_center_size(pos2(x, y), vec2(22.0, 22.0));
        let response = ui.interact(grip, Id::new("grip"), Sense::drag());
        dragged |= response.dragged();
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        }
        if response.dragged()
            && let Some(at) = pointer
        {
            board.layout_mut().set_column(share_x(at));
            board.layout_mut().set_row(share_y(at));
        }
        let painter = ui.painter();
        painter.circle_filled(grip.center(), 9.0, ui.visuals().window_fill);
        painter.circle_stroke(
            grip.center(),
            9.0,
            Stroke::new(1.0, ui.visuals().window_stroke.color),
        );
        for dx in [-2.5, 2.5] {
            painter.circle_filled(grip.center() + vec2(dx, 0.0), 1.4, p_dim);
        }
    }
    let between_columns = Rect::from_min_max(
        pos2(x - GAP / 2.0, area.top()),
        pos2(x + GAP / 2.0, area.bottom()),
    );
    let response = ui.interact(between_columns, Id::new("column-line"), Sense::drag());
    dragged |= response.dragged();
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if response.dragged()
        && let Some(at) = pointer
    {
        board.layout_mut().set_column(share_x(at));
    }
    if four || three {
        // Three panes: the line between rows runs along the right column.
        let left = if three { x + GAP / 2.0 } else { area.left() };
        let between_rows =
            Rect::from_min_max(pos2(left, y - GAP / 2.0), pos2(area.right(), y + GAP / 2.0));
        let response = ui.interact(between_rows, Id::new("row-line"), Sense::drag());
        dragged |= response.dragged();
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        }
        if response.dragged()
            && let Some(at) = pointer
        {
            board.layout_mut().set_row(share_y(at));
        }
    }
    dragged
}

/// One pane: its card, its title bar and what it shows.
fn pane(
    ui: &mut Ui,
    board: &mut Board,
    index: usize,
    rect: Rect,
    lit: f32,
    paint: &mut Paint<'_>,
    asked: &mut Vec<Request>,
) {
    let p = paint.p;
    let focused = index == board.layout().focus();
    let round = CornerRadius::same(ROUND);
    let painter = ui.painter().clone();
    let shadow = if focused {
        Shadow {
            offset: [0, 16],
            blur: 40,
            spread: 0,
            color: p.shadow,
        }
    } else {
        Shadow {
            offset: [0, 8],
            blur: 22,
            spread: 0,
            color: theme::alpha(p.shadow, 0.6),
        }
    };
    painter.add(shadow.as_shape(rect, round));
    painter.rect_filled(rect, round, if focused { p.pane_focus } else { p.pane });
    let edge = if focused {
        theme::alpha(p.accent, 0.45)
    } else {
        p.edge
    };
    painter.rect_stroke(rect, round, Stroke::new(1.0, edge), StrokeKind::Inside);
    if lit > 0.0 {
        // A message landed, or the pane just opened: its edge glows, then
        // fades; outside it only, the pane itself stays dark.
        painter.rect_stroke(
            rect,
            round,
            Stroke::new(1.5, theme::alpha(p.accent, 0.9 * lit)),
            StrokeKind::Inside,
        );
        for (out, share) in [(1.5, 0.35), (3.5, 0.18), (6.0, 0.08)] {
            painter.rect_stroke(
                rect.expand(out),
                CornerRadius::same(ROUND + out as u8),
                Stroke::new(2.0, theme::alpha(p.accent, share * lit)),
                StrokeKind::Middle,
            );
        }
    }
    if focused {
        theme::light_line(
            &painter,
            rect.left() + 16.0,
            rect.right() - 16.0,
            rect.top() + 0.5,
            theme::alpha(p.accent, 0.9),
        );
    }
    let head = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + HEAD));
    painter.line_segment(
        [
            pos2(head.left() + 1.0, head.bottom()),
            pos2(head.right() - 1.0, head.bottom()),
        ],
        Stroke::new(1.0, p.edge),
    );
    title(ui, board, index, head, paint);
    let body = Rect::from_min_max(
        pos2(rect.left() + 14.0, head.bottom() + 10.0),
        pos2(rect.right() - 14.0, rect.bottom() - 12.0),
    );
    let mut inside = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .layout(Layout::bottom_up(Align::Min))
            .id_salt(("pane-body", index)),
    );
    // What a pane holds stays within it, a wide table too, and while it
    // moves, when it is narrower than what it holds.
    inside.set_clip_rect(rect.intersect(ui.clip_rect()));
    let mut done = Done::default();
    let shows = board.layout().pane(index).map(|pane| pane.shows.clone());
    match shows {
        Some(Shows::Chat(agent)) => {
            let view = board.crew().agent(&agent).cloned();
            let hint = format!("Write to {agent}…");
            if let Some(text) = board.layout_mut().pane_mut(index).map(|p| &mut p.input) {
                chat::input(&mut inside, index, text, &hint, focused, p, &mut done);
            }
            inside.add_space(8.0);
            let questions: Vec<_> = board.questions_in(index).into_iter().cloned().collect();
            let keys = (chord(board, Action::Allow), chord(board, Action::Refuse));
            let stop = chord(board, Action::Stop);
            inside.with_layout(Layout::top_down(Align::Min), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("thread", index))
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height(ui.available_height())
                    .show(ui, |ui| {
                        match &view {
                            Some(view) => {
                                let working = view
                                    .working
                                    .then(|| paint.since.get(&view.name).map(Instant::elapsed))
                                    .flatten();
                                chat::thread(ui, view, working, &stop, paint.cache, p);
                            }
                            None => {
                                ui.label(RichText::new("This agent is gone.").color(p.dim));
                            }
                        }
                        let refs: Vec<_> = questions.iter().collect();
                        chat::questions(ui, &refs, (&keys.0, &keys.1), p, &mut done);
                    });
            });
        }
        Some(Shows::New(new)) => {
            let hint = format!("The task for {}…", short_model(&new.model));
            if let Some(text) = board.layout_mut().pane_mut(index).map(|p| &mut p.input) {
                chat::input(&mut inside, index, text, &hint, focused, p, &mut done);
            }
            inside.add_space(8.0);
            let lead = board.crew().lead().to_owned();
            let questions: Vec<_> = board.questions_in(index).into_iter().cloned().collect();
            let keys = (chord(board, Action::Allow), chord(board, Action::Refuse));
            inside.with_layout(Layout::top_down(Align::Min), |ui| {
                let refs: Vec<_> = questions.iter().collect();
                chat::questions(ui, &refs, (&keys.0, &keys.1), p, &mut done);
                // A small pane scrolls its form rather than draw it under
                // the input.
                egui::ScrollArea::vertical()
                    .id_salt(("new", index))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if let Some(iq_ui::layout::Pane {
                            shows: Shows::New(chat),
                            ..
                        }) = board.layout_mut().pane_mut(index)
                        {
                            new_chat(ui, chat, &lead, p);
                        }
                    });
            });
        }
        Some(Shows::Costs) => {
            inside.with_layout(Layout::top_down(Align::Min), |ui| {
                questions_only(ui, board, index, p, &mut done);
                egui::ScrollArea::vertical()
                    .id_salt(("costs", index))
                    .auto_shrink([false, false])
                    .show(ui, |ui| costs_page::show(ui, board, false, p));
            });
        }
        Some(Shows::Subscriptions) => {
            inside.with_layout(Layout::top_down(Align::Min), |ui| {
                questions_only(ui, board, index, p, &mut done);
                egui::ScrollArea::vertical()
                    .id_salt(("subscriptions", index))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        subscriptions_page::show(ui, board.subscriptions(), p)
                    });
            });
        }
        None => {}
    }
    for (id, approved) in done.answered {
        asked.push(board.answer(id, approved));
    }
    if done.sent
        && let Some(request) = board.send(index)
    {
        asked.push(request);
    }
}

/// The questions a pane that shows no chat puts to the person.
fn questions_only(ui: &mut Ui, board: &Board, index: usize, p: &Palette, done: &mut Done) {
    let questions = board.questions_in(index);
    if questions.is_empty() {
        return;
    }
    let keys = (chord(board, Action::Allow), chord(board, Action::Refuse));
    chat::questions(ui, &questions, (&keys.0, &keys.1), p, done);
    ui.add_space(8.0);
}

/// The key that does `action`, as a hint writes it.
fn chord(board: &Board, action: Action) -> String {
    board
        .keys()
        .chord(action)
        .map(ToString::to_string)
        .unwrap_or_default()
}

/// A model's name without the agent's: `opus` for `claude-code/opus`.
fn short_model(model: &str) -> &str {
    match model.split_once('/') {
        Some(("claude-code", rest)) => rest,
        _ => model,
    }
}

/// A pane's title bar: what it shows, which a click on its name changes,
/// and a double click anywhere on it to have it over the whole window.
fn title(ui: &mut Ui, board: &mut Board, index: usize, head: Rect, paint: &Paint<'_>) {
    let p = paint.p;
    let response = ui.interact(head, Id::new(("pane-head", index)), Sense::click());
    if response.double_clicked() {
        board.layout_mut().set_focus(index);
        board.layout_mut().toggle_zoom();
    }
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(head.shrink2(vec2(14.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center))
            .id_salt(("pane-title", index)),
    );
    bar.set_clip_rect(head.intersect(ui.clip_rect()));
    bar.spacing_mut().item_spacing.x = 8.0;
    // The title reads as a title, a button only under the pointer.
    bar.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    bar.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
    let shows = board.layout().pane(index).map(|pane| pane.shows.clone());
    let agent: Option<AgentView> = match &shows {
        Some(Shows::Chat(name)) => board.crew().agent(name).cloned(),
        _ => None,
    };
    let (dot, _) = bar.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    let time = bar.input(|i| i.time) as f32;
    let (colour, glow) = match &agent {
        Some(view) if view.working => (p.accent, 0.55 + 0.45 * (time * 2.4).sin()),
        Some(_) => (p.idle, 0.0),
        None => (theme::alpha(p.dim, 0.6), 0.0),
    };
    theme::sphere(bar.painter(), dot.center(), 4.6, colour, glow);
    let name = match &shows {
        Some(Shows::Chat(name)) => name.clone(),
        Some(Shows::New(chat)) => match &chat.starting {
            Some(name) => format!("{name}, starting"),
            None => "New chat".to_owned(),
        },
        Some(Shows::Costs) => "Costs".to_owned(),
        Some(Shows::Subscriptions) => "Subscriptions".to_owned(),
        None => String::new(),
    };
    let names: Vec<String> = board
        .crew()
        .agents()
        .iter()
        .map(|a| a.name.clone())
        .collect();
    let mut chosen = None;
    bar.menu_button(
        RichText::new(name).font(theme::bold(13.0)).color(p.text),
        |ui| {
            for agent in &names {
                if ui.button(agent).clicked() {
                    chosen = Some(Shows::Chat(agent.clone()));
                }
            }
            if !names.is_empty() {
                ui.separator();
            }
            if ui.button("New chat").clicked() {
                chosen = Some(Shows::New(NewChat {
                    model: board.crew().lead().to_owned(),
                    ..NewChat::default()
                }));
            }
            if ui.button("Costs").clicked() {
                chosen = Some(Shows::Costs);
            }
            if ui.button("Subscriptions").clicked() {
                chosen = Some(Shows::Subscriptions);
            }
        },
    );
    if let Some(view) = &agent {
        bar.label(
            RichText::new(&view.model)
                .font(theme::mono(MONO_SIZE - 0.8))
                .color(p.dim),
        );
        bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(spent(view))
                    .font(theme::mono(MONO_SIZE - 0.8))
                    .color(p.dim),
            );
        });
    }
    if let Some(shows) = chosen {
        board.layout_mut().show(index, shows);
        board.layout_mut().set_focus(index);
    }
}

/// What an agent's calls cost, as its title bar says it: dollars when
/// billed, tokens when a subscription paid, as it is paid by the month.
fn spent(agent: &AgentView) -> String {
    match (agent.billed.0 > 0.0, agent.subscription.0 > 0) {
        (true, true) => format!("{} · {} tokens", agent.billed, agent.subscription),
        (true, false) => agent.billed.to_string(),
        (false, true) => format!("{} tokens", agent.subscription),
        (false, false) => String::new(),
    }
}

/// A chat not started yet: its model, typed or picked, and what to do.
fn new_chat(ui: &mut Ui, chat: &mut NewChat, lead: &str, p: &Palette) {
    ui.add_space(6.0);
    if let Some(name) = &chat.starting {
        let phase = (ui.input(|i| i.time) % 2.4 / 2.4) as f32;
        ui.label(theme::shimmer(
            &format!("Starting {name}…"),
            phase,
            p.dim,
            p.accent,
        ));
        return;
    }
    ui.label(
        RichText::new("A new chat")
            .font(theme::bold(16.0))
            .color(p.text),
    );
    ui.label(
        RichText::new("It works in this folder, through iq's gate and sandbox.")
            .small()
            .color(p.dim),
    );
    ui.add_space(10.0);
    ui.label(RichText::new("Its model").small().color(p.dim));
    ui.add(
        egui::TextEdit::singleline(&mut chat.model)
            .font(theme::mono(MONO_SIZE))
            .desired_width(f32::INFINITY)
            .hint_text("a model of the provider, claude-code, codex"),
    );
    ui.add_space(4.0);
    models(ui, chat, lead, p);
    ui.add_space(10.0);
    ui.label(RichText::new("How it works").small().color(p.dim));
    ui.horizontal_wrapped(|ui| {
        for pattern in iq_ui::Pattern::ALL {
            let on = chat.pattern == pattern;
            let text = RichText::new(pattern.name())
                .font(theme::mono(MONO_SIZE - 1.0))
                .color(if on { p.accent } else { p.dim });
            if ui
                .selectable_label(on, text)
                .on_hover_text(pattern.what())
                .clicked()
            {
                chat.pattern = pattern;
            }
        }
    });
    ui.label(RichText::new(chat.pattern.what()).small().color(p.dim));
    if chat.pattern.takes_partner() {
        let what = if chat.pattern == iq_ui::Pattern::Pair {
            "Its reviewer's model"
        } else {
            "The second voice's model, the same when empty"
        };
        ui.label(RichText::new(what).small().color(p.dim));
        ui.add(
            egui::TextEdit::singleline(&mut chat.partner)
                .font(theme::mono(MONO_SIZE))
                .desired_width(f32::INFINITY)
                .hint_text("claude-code/haiku, codex, a model of the provider"),
        );
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new("Its role, if it has one")
            .small()
            .color(p.dim),
    );
    ui.add(
        egui::TextEdit::singleline(&mut chat.role)
            .desired_width(f32::INFINITY)
            .hint_text("tests, review, the front end… a word names it"),
    );
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for role in ["lead", "tests", "review", "docs", "front", "back"] {
            let on = chat.role.trim() == role;
            let text = RichText::new(role)
                .font(theme::mono(MONO_SIZE - 1.0))
                .color(if on { p.accent } else { p.dim });
            if ui.selectable_label(on, text).clicked() {
                chat.role = if on { String::new() } else { role.to_owned() };
            }
        }
    });
}

/// The models offered for a chat: the lead's, Claude Code's, Codex.
fn models(ui: &mut Ui, chat: &mut NewChat, lead: &str, p: &Palette) {
    // The lead's model is offered once, first, even when it is one of the
    // others.
    let mut offered: Vec<&str> = Vec::new();
    for model in [
        lead,
        "claude-code/opus",
        "claude-code/sonnet",
        "claude-code/haiku",
        "codex",
    ] {
        if !model.is_empty() && !offered.contains(&model) {
            offered.push(model);
        }
    }
    ui.horizontal_wrapped(|ui| {
        for model in offered {
            let on = chat.model.trim() == model;
            let text = RichText::new(model)
                .font(theme::mono(MONO_SIZE - 1.0))
                .color(if on { p.accent } else { p.dim });
            if ui.selectable_label(on, text).clicked() {
                chat.model = model.to_owned();
            }
        }
    });
}
