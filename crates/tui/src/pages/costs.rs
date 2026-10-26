//! Where the money goes: the limits the steward holds, what each payer
//! spent over the period (the provider's bill in dollars, a subscription's
//! share of its windows, since it is paid by the month), the provider's
//! bill as it grew, each model's calls and each kind of task's; then each
//! subscription's windows, its requests and the context its sessions read.

use iq_core::{TokenCount, Usd};
use iq_ui::PERIODS;
use iq_ui::costs::{Payer, Total};
use iq_ui::steward::Level;
use iq_ui::subscriptions::{WindowView, back_at, local_time, percent, short as agent_name, span};
use ratatui::text::{Line, Span};

use super::Seen;
use crate::chart::{Curve, gauge, sparkline};
use crate::theme::Rgb;
use crate::wrap::cut;

/// How many requests are listed.
const REQUESTS: usize = 16;

/// How many sessions' contexts are drawn.
const SESSIONS: usize = 6;

/// A period's name: `1h`, `24h`, `7d`.
fn period_name(seconds: u64) -> String {
    if seconds >= 2 * 24 * 3600 {
        format!("{}d", seconds / (24 * 3600))
    } else {
        format!("{}h", seconds / 3600)
    }
}

/// The page's lines.
pub(super) fn lines(seen: &Seen<'_>) -> Vec<Line<'static>> {
    let p = seen.p;
    let board = seen.board;
    let now = seen.now;
    let since = now.saturating_sub(board.period());
    let everyone = board.everyone();
    let mut lines = Vec::new();
    let mut periods = vec![Span::styled(" Over the last ", p.fg(p.dim))];
    for period in PERIODS {
        let on = board.period() == period;
        periods.push(Span::styled(
            format!(" {} ", period_name(period)),
            if on { p.bold(p.accent) } else { p.fg(p.faint) },
        ));
    }
    periods.push(Span::styled("    ", p.fg(p.faint)));
    periods.push(Span::styled(
        if everyone {
            "iq's calls and your own sessions"
        } else {
            "iq's calls"
        },
        p.fg(p.text),
    ));
    lines.push(Line::default());
    lines.push(Line::from(periods));
    limits(seen, &mut lines);
    if !board.was_read() {
        seen.said(&mut lines, "The agents' calls are being read…", p.dim);
        return lines;
    }
    let totals = board.costs().totals(since, everyone);
    if totals.is_empty() {
        lines.push(Line::default());
        let who = if everyone {
            "No call"
        } else {
            "iq made no call"
        };
        seen.said(
            &mut lines,
            &format!(
                "{who} to a model in the last {}.",
                period_name(board.period())
            ),
            p.dim,
        );
    } else {
        payers(seen, &totals, &mut lines);
        running(seen, since, &mut lines);
        by_model(seen, &totals, &mut lines);
    }
    by_kind(seen, since, &mut lines);
    subscriptions(seen, &mut lines);
    lines
}

/// Each limit the person set: how much is used, by iq and outside it, the
/// mark where an even pace would be, and when it is full at this pace.
fn limits(seen: &Seen<'_>, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let board = seen.board;
    let gauges = board.gauges();
    if gauges.is_empty() {
        return;
    }
    let word = if board.steward().on() {
        "the steward writes to who spends too fast"
    } else {
        "the steward only watches"
    };
    seen.heading(lines, "Limits", word);
    let name_width = gauges
        .iter()
        .map(|g| g.name().chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let bar = seen.width.saturating_sub(name_width + 40).clamp(10, 40);
    for g in gauges {
        let tint = match g.level {
            Level::Calm => p.ok,
            Level::Watch => p.warn,
            Level::Alert | Level::Reached => p.bad,
        };
        let limit = g.limit.max(f64::EPSILON);
        let mut spans = vec![Span::styled(
            format!("  {:<name_width$}", g.name()),
            p.fg(p.text),
        )];
        spans.extend(gauge(
            g.used / limit,
            Some(g.elapsed),
            bar,
            p.payer(g.payer),
            p,
        ));
        spans.push(Span::styled(
            format!("  {} of {}", g.amount(g.used), g.amount(g.limit)),
            p.fg(p.text),
        ));
        let below = match (g.level, g.full_at) {
            (Level::Reached, _) => "reached: closed".to_owned(),
            (Level::Calm, _) => "calm".to_owned(),
            (_, Some(at)) => format!("full {}", local_time(at)),
            (_, None) => "ahead of pace".to_owned(),
        };
        spans.push(Span::styled(format!("  {below}"), p.fg(tint)));
        if g.external > 0.05 {
            spans.push(Span::styled(
                format!(" · {} outside iq", g.amount(g.external)),
                p.fg(p.faint),
            ));
        }
        lines.push(Line::from(spans));
    }
    let letters = board.letters();
    if !letters.is_empty() {
        lines.push(Line::default());
        for letter in letters.iter().rev().take(5) {
            lines.push(Line::from(vec![
                Span::styled(format!("  {} → {}  ", letter.from, letter.to), p.fg(p.info)),
                Span::styled(
                    cut(
                        &letter.text.replace('\n', " "),
                        seen.width.saturating_sub(24),
                    ),
                    p.fg(p.dim),
                ),
            ]));
        }
    }
}

/// Each payer's period in large: the provider's bill, or how much of a
/// subscription's windows the calls took.
fn payers(seen: &Seen<'_>, totals: &[Total], lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let board = seen.board;
    let windows = board.subscriptions().windows(seen.now);
    let iq_only = !board.everyone();
    let mut payers: Vec<Payer> = totals.iter().map(|t| t.payer).collect();
    payers.sort();
    payers.dedup();
    lines.push(Line::default());
    for payer in payers {
        let mine: Vec<&Total> = totals.iter().filter(|t| t.payer == payer).collect();
        let calls: usize = mine.iter().map(|t| t.calls).sum();
        let tokens: u64 = mine.iter().map(|t| t.tokens).sum();
        let tint = p.payer(payer);
        let mut spans = vec![Span::styled(format!("  {:<12}", payer.name()), p.fg(p.dim))];
        match payer {
            Payer::Provider | Payer::Key(_) => {
                let mut cost = Usd(0.0);
                for total in &mine {
                    cost += total.cost;
                }
                spans.push(Span::styled(
                    format!("{:<12}", cost.to_string()),
                    p.bold(tint),
                ));
                spans.push(Span::styled(
                    format!("billed · {calls} calls"),
                    p.fg(p.faint),
                ));
            }
            Payer::Subscription(agent) => {
                let shares: Vec<String> = windows
                    .iter()
                    .filter(|w| w.agent == agent)
                    .map(|w| {
                        let part = if iq_only {
                            board.subscriptions().iq_part(w)
                        } else {
                            None
                        };
                        window_share(w, part)
                    })
                    .collect();
                let first = shares
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "subscription".to_owned());
                spans.push(Span::styled(format!("{first:<12}"), p.bold(tint)));
                if shares.len() > 1 {
                    spans.push(Span::styled(
                        format!("{}  ", shares[1..].join(" · ")),
                        p.fg(p.text),
                    ));
                }
                spans.push(Span::styled(
                    format!("{calls} calls · {} tokens", TokenCount(tokens)),
                    p.fg(p.faint),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
}

/// How much of a window is used, `42% of 5h`: iq's `part` when known.
fn window_share(window: &WindowView, part: Option<f64>) -> String {
    let used = match (part, window.estimate) {
        (Some(part), _) => format!("~{}", percent(part)),
        (None, Some(estimate)) => format!("~{}", percent(estimate)),
        (None, None) => percent(window.reported),
    };
    format!("{used} of {}", span(window.minutes))
}

/// What the provider billed, summed as time went.
fn running(seen: &Seen<'_>, since: u64, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let board = seen.board;
    let now = seen.now;
    let Some((_, points)) = board
        .costs()
        .running(since, board.everyone())
        .into_iter()
        .find(|(payer, _)| *payer == Payer::Provider)
    else {
        return;
    };
    let mut points: Vec<(f64, f64)> = points
        .iter()
        .map(|(at, sum)| (-(now.saturating_sub(*at) as f64) / 3600.0, *sum))
        .collect();
    if let Some(&(_, last)) = points.last() {
        points.push((0.0, last));
    }
    let top = points.iter().map(|(_, v)| *v).fold(0.0, f64::max).max(0.01);
    let hours = board.period() as f64 / 3600.0;
    seen.heading(lines, "The provider's bill", "as it grew over the period");
    let label = |v: f64| format!("${v:.2}");
    lines.extend(
        Curve {
            points: &points,
            x: (-hours, 0.0),
            y: (0.0, top),
            width: seen.width.saturating_sub(2).min(100),
            height: 5,
            tint: p.ok,
            label: &label,
            ends: (
                format!("{} ago", period_name(board.period())),
                "now".to_owned(),
            ),
        }
        .lines(p)
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(line.spans);
            Line::from(spans)
        }),
    );
}

/// Each payer's calls by model.
fn by_model(seen: &Seen<'_>, totals: &[Total], lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    seen.heading(lines, "By model", "");
    let model_width = totals
        .iter()
        .map(|t| t.model.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    for total in totals {
        let cost = match total.payer {
            Payer::Provider | Payer::Key(_) => {
                let tint = if total.unpriced > 0 { p.warn } else { p.text };
                let mut said = total.cost.to_string();
                if total.unpriced > 0 {
                    said.push_str(&format!(" ({} without a known price)", total.unpriced));
                }
                Span::styled(said, p.fg(tint))
            }
            Payer::Subscription(_) => {
                Span::styled(format!("{} tokens", TokenCount(total.tokens)), p.fg(p.dim))
            }
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<12}", total.payer.name()),
                p.fg(p.payer(total.payer)),
            ),
            Span::styled(format!("{:<model_width$}", total.model), p.fg(p.text)),
            Span::styled(format!("{:>6} calls  ", total.calls), p.fg(p.faint)),
            cost,
        ]));
    }
    seen.said(
        lines,
        "A subscription is paid by the month: what counts is how much of its windows the calls take, not dollars.",
        p.faint,
    );
}

/// What each kind of task spent over the period.
fn by_kind(seen: &Seen<'_>, since: u64, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let kinds = seen.board.costs().by_kind(since);
    if kinds.is_empty() {
        return;
    }
    seen.heading(
        lines,
        "By kind of task",
        "as your categories place iq's tasks",
    );
    let most = kinds
        .iter()
        .map(|k| k.billed.0 + k.listed.0)
        .fold(0.0, f64::max);
    let name_width = kinds
        .iter()
        .map(|k| k.kind.as_deref().unwrap_or("no kind").chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    for kind in &kinds {
        let name = kind.kind.as_deref().unwrap_or("no kind");
        let share = if most > 0.0 {
            (kind.billed.0 + kind.listed.0) / most
        } else {
            0.0
        };
        let mut spans = vec![
            Span::styled(
                format!("  {name:<name_width$}"),
                p.fg(if kind.kind.is_some() { p.text } else { p.dim }),
            ),
            Span::styled(format!("{:>4} tasks  ", kind.tasks), p.fg(p.faint)),
            Span::styled(format!("{:<10}", kind.billed.to_string()), p.fg(p.text)),
            Span::styled(
                format!("{:<16}", format!("{} listed", kind.listed)),
                p.fg(p.dim),
            ),
        ];
        spans.extend(gauge(share, None, 16, p.accent, p));
        lines.push(Line::from(spans));
    }
}

/// Each subscription's windows, its requests, its sessions' contexts.
fn subscriptions(seen: &Seen<'_>, lines: &mut Vec<Line<'static>>) {
    let p = seen.p;
    let subscriptions = seen.board.subscriptions();
    let now = seen.now;
    seen.heading(
        lines,
        "Subscriptions",
        "whole percents as the agents report them; after ~, estimated from each call's tokens",
    );
    let windows = subscriptions.windows(now);
    if windows.is_empty() {
        seen.said(
            lines,
            "Nothing heard yet: a subscription's windows show once Claude Code or Codex answers under it.",
            p.dim,
        );
        return;
    }
    let bar = seen.width.saturating_sub(60).clamp(10, 30);
    for window in &windows {
        let share = window.estimate.unwrap_or(window.reported);
        let tint = fill(share, p);
        let shown = match window.estimate {
            Some(estimate) => format!("~{}", percent(estimate)),
            None => percent(window.reported),
        };
        let back = match window.resets_at {
            Some(at) => format!("starts again {}", back_at(at, window.minutes)),
            None => "ended: starts again at the next use".to_owned(),
        };
        let mut spans = vec![
            Span::styled(
                format!(
                    "  {:<14}",
                    format!("{} · {}", agent_name(window.agent), span(window.minutes))
                ),
                p.fg(p.text),
            ),
            Span::styled(format!("{shown:<7}"), p.bold(tint)),
        ];
        spans.extend(gauge(share / 100.0, None, bar, tint, p));
        spans.push(Span::styled(format!("  {back}"), p.fg(p.faint)));
        lines.push(Line::from(spans));
        if let Some(end) = window.resets_at
            && window.reports.len() > 1
        {
            let start = end.saturating_sub(window.minutes * 60);
            let hours = |at: u64| at.saturating_sub(start) as f64 / 3600.0;
            let points: Vec<(f64, f64)> = window
                .reports
                .iter()
                .map(|(at, used)| (hours(*at), *used))
                .collect();
            let top = points
                .iter()
                .map(|(_, v)| *v)
                .fold(window.estimate.unwrap_or(0.0), f64::max)
                .max(5.0)
                * 1.2;
            let label = |v: f64| format!("{v:.0}%");
            lines.extend(
                Curve {
                    points: &points,
                    x: (0.0, window.minutes as f64 / 60.0),
                    y: (0.0, top),
                    width: seen.width.saturating_sub(16).min(80),
                    height: 3,
                    tint: p.info,
                    label: &label,
                    ends: ("its start".to_owned(), format!("{}h", window.minutes / 60)),
                }
                .lines(p)
                .into_iter()
                .map(|line| {
                    let mut spans = vec![Span::raw("                ")];
                    spans.extend(line.spans);
                    Line::from(spans)
                }),
            );
        }
    }
    let requests = subscriptions.requests(REQUESTS);
    if !requests.is_empty() {
        seen.heading(lines, "Requests", "the latest first, and what each took");
        for request in &requests {
            let folder = request
                .folder
                .rsplit('/')
                .next()
                .unwrap_or(&request.folder)
                .to_owned();
            let who = if request.by_iq { "iq" } else { "you" };
            let shares: Vec<String> = request
                .shares
                .iter()
                .map(|(_, minutes, share)| {
                    let share = share.map_or_else(|| "?".to_owned(), |s| format!("{s:.2}%"));
                    format!("{} {share}", span(*minutes))
                })
                .collect();
            let mut spans = vec![
                Span::styled(format!("  {}  ", local_time(request.at)), p.fg(p.faint)),
                Span::styled(format!("{:<12}", agent_name(request.agent)), p.fg(p.text)),
                Span::styled(format!("{}  ", request.models.join(", ")), p.fg(p.text)),
                Span::styled(format!("{folder} · {who}  "), p.fg(p.dim)),
            ];
            if !shares.is_empty() {
                spans.push(Span::styled(
                    format!("{}  ", shares.join(" · ")),
                    p.fg(p.info),
                ));
            }
            spans.push(Span::styled(
                format!(
                    "{} call{} · {} in · {} cached · {} out",
                    request.calls,
                    if request.calls == 1 { "" } else { "s" },
                    TokenCount(request.input),
                    TokenCount(request.cache_read),
                    TokenCount(request.output)
                ),
                p.fg(p.faint),
            ));
            if let Some(tokens) = request.cache_gone {
                spans.push(Span::styled(
                    format!("  cache gone: {} read again", TokenCount(tokens)),
                    p.fg(p.warn),
                ));
            }
            lines.push(Line::from(spans));
        }
    }
    let sessions = subscriptions.sessions(SESSIONS);
    if !sessions.is_empty() {
        seen.heading(lines, "Sessions", "the context each call read");
        for session in sessions {
            let folder = session
                .folder
                .rsplit('/')
                .next()
                .unwrap_or(&session.folder)
                .to_owned();
            let values: Vec<f64> = session.context.iter().map(|(_, c)| *c as f64).collect();
            let last = session.context.last().map_or(0, |(_, c)| *c);
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "  {:<44}",
                        cut(
                            &format!(
                                "{} · {} · {folder} · {} calls",
                                agent_name(session.agent),
                                session.model,
                                session.context.len()
                            ),
                            43
                        )
                    ),
                    p.fg(p.text),
                ),
                sparkline(&values, 32, p.accent, p),
                Span::styled(format!("  {}", TokenCount(last)), p.fg(p.faint)),
            ]));
        }
    }
}

/// How full a share is, as a colour.
fn fill(share: f64, p: &crate::theme::Palette) -> Rgb {
    if share >= 90.0 {
        p.bad
    } else if share >= 70.0 {
        p.warn
    } else {
        p.info
    }
}
