//! The conversation iq draws itself, as the settings ask (`conversation =
//! "drawn"`): on the terminal's other screen, the mouse taken, from the
//! blocks each chat's window keeps, laid out again at every draw, so that
//! what is folded (a fold of steps, a job that ended, a message between
//! agents, a diagram folded) unfolds in place, alone, with a click or
//! Enter on the mark, and folds again the same, nothing moving but what
//! follows it. A diagram is enlarged with a click, across the width, the
//! conversation's room its alone and the wheel its, until another click
//! puts it back. iq finds words in it itself, each match marked in turn,
//! and copies what is marked as it was written. A link opens with a click,
//! its address shown while the mouse is over it, or with a key from what
//! is marked, iq handing it to the system's opener as the terminal would.
//! An answer is read whole one after another: back to its top from the
//! mark after it when it is taller than the room, with Home, End and Shift
//! with the arrows. The end follows what is written while the person is
//! at it; scrolled up, the view stays put as more comes, its top held by
//! the block it is in.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iq_ui::steps::Entry;
use iq_ui::{Board, Pointed, Reading};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::diagram::Image;
use crate::history::{Block, History, Speaker};
use crate::pages::steps::body;
use crate::theme::Palette;

/// What iq opens with, before the first block, as a place of the view.
const OPENING: usize = usize::MAX;

/// The conversation as iq draws it, and where the person reads it.
#[derive(Debug, Default)]
pub(crate) struct View {
    /// The chat whose window was drawn last: another starts at its end,
    /// all folded.
    window: Option<Option<String>>,
    /// The view's top: the block it is in, by its number ([`OPENING`] for
    /// what iq opens with), and the row in it; none while it follows the
    /// end.
    top: Option<(usize, usize)>,
    /// The blocks unfolded in place, by their numbers.
    unfolded: HashSet<usize>,
    /// What is marked: what Enter unfolds or folds, or the mark after an
    /// answer, which goes back to its top.
    mark: Option<Spot>,
    /// The diagram enlarged, by its block's number, and its first row
    /// shown.
    zoom: Option<(usize, usize)>,
    /// What the person looks for, and the blocks that hold it, by their
    /// numbers, the oldest first.
    found: Option<(String, Vec<usize>)>,
    /// What each row of the conversation showed, as last drawn.
    rows: Vec<Option<Spot>>,
    /// The links each row showed, as last drawn.
    links: Vec<Vec<Link>>,
    /// The screen's row the conversation began at, as last drawn.
    from: u16,
    /// Where the mouse is, its column and its row on the screen.
    pointer: Option<(u16, u16)>,
    /// The links of what is marked, listed for the person to choose one.
    listing: Option<Listing>,
}

/// What a row of the conversation shows, or the mark is on: a block, by
/// its number, or the mark after an agent's answer taller than the room,
/// which goes back to its top, by the number of the block it follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    Block(usize),
    Back(usize),
}

impl Spot {
    /// The block it is, when it is one.
    fn block(self) -> Option<usize> {
        match self {
            Spot::Block(number) => Some(number),
            Spot::Back(_) => None,
        }
    }

    /// The number of the block it is, or follows.
    fn number(self) -> usize {
        match self {
            Spot::Block(number) | Spot::Back(number) => number,
        }
    }

    /// Where it stands among what is in one place: the block, then the
    /// mark after it.
    fn rank(self) -> u8 {
        match self {
            Spot::Block(_) => 1,
            Spot::Back(_) => 2,
        }
    }
}

/// A link where the conversation shows it: its columns on its row, and
/// its address.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Link {
    from: usize,
    to: usize,
    number: usize,
    address: String,
}

/// The links of what is marked, each its text and its address, and the
/// one chosen.
#[derive(Debug)]
struct Listing {
    links: Vec<(String, String)>,
    picked: usize,
}

/// What the conversation shows, laid out.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    /// Its rows, as many as its height at most.
    pub(crate) lines: Vec<Line<'static>>,
    /// The images they show, for the terminal to be sent first.
    pub(crate) images: Vec<Image>,
    /// Whether a click fell outside it: the person reads it no more.
    pub(crate) missed: bool,
    /// What the person copied, as it was written, for the terminal's
    /// clipboard.
    pub(crate) copied: Option<String>,
    /// What to tell the person of what they asked: what was found, what
    /// was copied.
    pub(crate) said: Option<String>,
    /// The address of the link the person opened, for the system's opener.
    pub(crate) open: Option<String>,
    /// The link the person points at, for the status line.
    pub(crate) pointed: Option<Pointed>,
}

/// Where the conversation is drawn: the screen's rows from `from`, as many
/// as `height`, within `width` columns.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Area {
    pub(crate) from: u16,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// The blocks of a window, each with its number, after what iq opens with.
struct Laid<'a> {
    opening: Vec<Line<'static>>,
    first: usize,
    blocks: Vec<&'a Block>,
    width: usize,
    /// The conversation's room, in rows.
    height: usize,
    cell: (u16, u16),
    board: &'a Board,
    p: &'a Palette,
    unfolded: &'a HashSet<usize>,
    /// How many rows each place's block takes, by the place, once counted.
    counted: RefCell<HashMap<usize, usize>>,
}

impl Laid<'_> {
    /// How many places there are: what iq opens with, then each block.
    fn len(&self) -> usize {
        1 + self.blocks.len()
    }

    /// The place `at` holds, by its number.
    fn number(&self, at: usize) -> usize {
        if at == 0 {
            OPENING
        } else {
            self.first + at - 1
        }
    }

    /// Where the block numbered `number` is; the first block kept for one
    /// let go since.
    fn at(&self, number: usize) -> usize {
        if number == OPENING {
            0
        } else {
            (number.saturating_sub(self.first) + 1).min(self.len() - 1)
        }
    }

    /// Whether what place `at` holds folds and unfolds.
    fn foldable(&self, at: usize) -> bool {
        at > 0
            && matches!(
                self.blocks[at - 1],
                Block::Entry { .. } | Block::Folded { .. }
            )
    }

    /// The rows of the diagram place `at` holds, enlarged; none for
    /// anything else, or a diagram that shows no image.
    fn enlarged(&self, at: usize, images: &mut Vec<Image>) -> Option<Vec<Line<'static>>> {
        match self.blocks.get(at.checked_sub(1)?)? {
            Block::Diagram { diagram, .. } => diagram.enlarged(self.width, self.cell, images),
            _ => None,
        }
    }

    /// The blocks whose rows hold `words`, whatever their case, by their
    /// numbers, the oldest first.
    fn holding(&self, words: &str) -> Vec<usize> {
        let words = words.to_lowercase();
        (1..self.len())
            .filter(|at| {
                self.block_lines(*at, &mut Vec::new())
                    .iter()
                    .any(|line| line.to_string().to_lowercase().contains(&words))
            })
            .map(|at| self.number(at))
            .collect()
    }

    /// What place `at` says, as it was written, for the person to copy: a
    /// fold's steps whole, a message whole, a job's command and output, a
    /// block of code or a paragraph of text whole, anything else as it is.
    fn copied(&self, at: usize) -> Option<String> {
        let block = self.blocks.get(at.checked_sub(1)?)?;
        let same = |other: &Block| match (block, other) {
            (Block::Whole(_), Block::Whole(_)) => true,
            (Block::Text(_), Block::Text(_)) => !other.written().trim().is_empty(),
            _ => false,
        };
        match block {
            Block::Entry { id, .. } => Some(match self.board.steps().entry(*id)? {
                Entry::Fold(fold) => fold
                    .steps
                    .iter()
                    .map(iq_ui::steps::Step::whole)
                    .collect::<Vec<_>>()
                    .join("\n"),
                Entry::Said(said) => said.text.clone(),
                Entry::Ran(ran) => format!("{}\n{}", ran.line, ran.output),
            }),
            Block::Whole(_) | Block::Text(_) => {
                let at = at - 1;
                let first = (0..=at)
                    .rev()
                    .take_while(|place| same(self.blocks[*place]))
                    .last()
                    .unwrap_or(at);
                let last = (at..self.blocks.len())
                    .take_while(|place| same(self.blocks[*place]))
                    .last()
                    .unwrap_or(at);
                // A block of code without the marks above and below it.
                let written: Vec<String> = self.blocks[first..=last]
                    .iter()
                    .map(|block| block.written())
                    .filter(|line| !line.starts_with(['╭', '╰']))
                    .collect();
                Some(written.join("\n"))
            }
            other => Some(other.written()),
        }
    }

    /// The links place `at` shows, each its text and its address, in the
    /// order they show: a link folded over lines is one.
    fn links(&self, at: usize) -> Vec<(String, String)> {
        let mut links: Vec<(usize, String, String)> = Vec::new();
        for line in self.block_lines(at, &mut Vec::new()) {
            let mut last = None;
            for span in &line.spans {
                let Some((number, address, _)) = crate::link::address_of(span.style) else {
                    last = None;
                    continue;
                };
                match links.iter_mut().find(|(n, ..)| *n == number) {
                    // Its next span on the line, or the next line of it.
                    Some((_, text, _)) => {
                        if last != Some(number) {
                            text.push(' ');
                        }
                        text.push_str(&span.content);
                    }
                    None => links.push((number, span.content.to_string(), address)),
                }
                last = Some(number);
            }
        }
        links
            .into_iter()
            .map(|(_, text, address)| (text.trim().to_owned(), address))
            .collect()
    }

    /// Whether the mark goes to place `at`: what folds, a diagram.
    fn markable(&self, at: usize) -> bool {
        self.foldable(at) || at > 0 && matches!(self.blocks[at - 1], Block::Diagram { .. })
    }

    /// The rows of place `at`, the mark back to its answer's top after it
    /// when it ends one taller than the room, the images they show put in
    /// `images`.
    fn lines(&self, at: usize, images: &mut Vec<Image>) -> Vec<Line<'static>> {
        let mut lines = self.block_lines(at, images);
        if self.ends_tall(at) {
            let p = self.p;
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("↑ top of this answer", p.fg(p.faint)),
            ]));
        }
        lines
    }

    /// What each of `count` rows of place `at` shows: its block, the last
    /// the mark back to its answer's top when it ends one taller than the
    /// room; nothing for what iq opens with.
    fn spots(&self, at: usize, count: usize) -> Vec<Option<Spot>> {
        if at == 0 {
            return vec![None; count];
        }
        let number = self.number(at);
        let mut spots = vec![Some(Spot::Block(number)); count];
        if self.ends_tall(at)
            && let Some(last) = spots.last_mut()
        {
            *last = Some(Spot::Back(number));
        }
        spots
    }

    /// Whose turn place `at` begins, when it begins one.
    fn head(&self, at: usize) -> Option<Speaker> {
        self.blocks.get(at.checked_sub(1)?)?.head()
    }

    /// Whether place `at` is a line left empty.
    fn blank(&self, at: usize) -> bool {
        at > 0 && self.blocks[at - 1].blank()
    }

    /// Where the turn holding place `at` begins: what iq opens with when
    /// none does.
    fn head_of(&self, at: usize) -> usize {
        (1..=at)
            .rev()
            .find(|place| self.head(*place).is_some())
            .unwrap_or(0)
    }

    /// The last place of the turn holding place `at`, the lines left empty
    /// before the next aside.
    fn end_of(&self, at: usize) -> usize {
        let next = (at + 1..self.len())
            .find(|place| self.head(*place).is_some())
            .unwrap_or(self.len());
        (at..next)
            .rev()
            .find(|place| !self.blank(*place))
            .unwrap_or(at)
    }

    /// Whether place `at` ends an agent's answer taller than the room,
    /// which the mark back to its top then follows.
    fn ends_tall(&self, at: usize) -> bool {
        if at == 0 || self.blank(at) {
            return false;
        }
        let mut next = at + 1;
        while next < self.len() && self.blank(next) {
            next += 1;
        }
        if next < self.len() && self.head(next).is_none() {
            return false;
        }
        let mut rows = 0;
        for place in (1..=at).rev() {
            rows += self.block_rows(place);
            if let Some(speaker) = self.head(place) {
                return speaker == Speaker::Agent && rows > self.height;
            }
            if rows > self.height {
                return self.head(self.head_of(place)) == Some(Speaker::Agent);
            }
        }
        false
    }

    /// How many rows place `at`'s block takes, counted once.
    fn block_rows(&self, at: usize) -> usize {
        if let Some(rows) = self.counted.borrow().get(&at) {
            return *rows;
        }
        let rows = self.block_lines(at, &mut Vec::new()).len();
        self.counted.borrow_mut().insert(at, rows);
        rows
    }

    /// The rows of place `at`'s block, unfolded when the person unfolded
    /// it, the images they show put in `images`.
    fn block_lines(&self, at: usize, images: &mut Vec<Image>) -> Vec<Line<'static>> {
        if at == 0 {
            return self.opening.clone();
        }
        let block = self.blocks[at - 1];
        if self.unfolded.contains(&self.number(at))
            && let Some(lines) = self.unfolded(block, images)
        {
            return lines;
        }
        block.lines(self.width, self.cell, images)
    }

    /// `block` unfolded: a fold of several steps under its count, each
    /// whole with what its tools returned; one step whole; a job under its
    /// line, its command whole and the end of its output; a message as it
    /// was written; a diagram folded, its image.
    fn unfolded(&self, block: &Block, images: &mut Vec<Image>) -> Option<Vec<Line<'static>>> {
        match block {
            Block::Entry { id, blocks } => {
                let entry = self.board.steps().entry(*id)?;
                let whole = body(&entry, true, self.p, self.width, &|_| None);
                let above = |count: usize, images: &mut Vec<Image>| -> Vec<Line<'static>> {
                    blocks
                        .iter()
                        .take(count)
                        .flat_map(|block| block.lines(self.width, self.cell, images))
                        .collect()
                };
                let mut lines = match entry {
                    Entry::Fold(fold) if fold.steps.len() > 1 => above(1, images),
                    Entry::Ran(_) => above(blocks.len(), images),
                    Entry::Fold(_) | Entry::Said(_) => Vec::new(),
                };
                lines.extend(whole);
                Some(lines)
            }
            Block::Folded {
                diagram,
                code,
                failed,
                ..
            } => Some(
                Block::Diagram {
                    diagram: Arc::clone(diagram),
                    code: code.clone(),
                    failed: *failed,
                }
                .lines(self.width, self.cell, images),
            ),
            _ => None,
        }
    }

    /// How many rows place `at` takes.
    fn rows(&self, at: usize) -> usize {
        self.block_rows(at) + usize::from(self.ends_tall(at))
    }
}

impl View {
    /// What the conversation of `window` shows in `area`, once `readings`,
    /// what the person asked of it, are carried out.
    pub(crate) fn shown(
        &mut self,
        history: &History,
        board: &Board,
        p: &Palette,
        window: Option<&str>,
        area: Area,
        readings: &[Reading],
    ) -> Shown {
        let key = window.map(str::to_owned);
        if self.window.as_ref() != Some(&key) {
            self.window = Some(key);
            self.top = None;
            self.mark = None;
            self.zoom = None;
            self.found = None;
            self.unfolded.clear();
        }
        let height = area.height.max(1);
        let mut missed = false;
        let mut copied = None;
        let mut said = None;
        let mut open = None;
        let (first, blocks) = history.blocks(window);
        let blocks: Vec<&Block> = blocks.into_iter().flatten().collect();
        for reading in readings {
            let unfolded = self.unfolded.clone();
            let laid = Laid {
                opening: history.opening_lines(area.width),
                first,
                blocks: blocks.clone(),
                width: area.width,
                height,
                cell: history.cell(),
                board,
                p,
                unfolded: &unfolded,
                counted: RefCell::default(),
            };
            // Enlarged, a diagram takes the wheel; a click, Enter or
            // anything else puts it back.
            // Listed, the links take the arrows, Enter and the numbers;
            // anything else but the wheel puts them away.
            if let Some(listing) = &mut self.listing {
                let last = listing.links.len().saturating_sub(1);
                let chosen = match reading {
                    Reading::Mark(by) => {
                        listing.picked = listing.picked.saturating_add_signed(*by).min(last);
                        continue;
                    }
                    Reading::Toggle | Reading::Open => {
                        Some(listing.links.get(listing.picked).cloned())
                    }
                    Reading::Pick(number) => Some(
                        number
                            .checked_sub(1)
                            .and_then(|at| listing.links.get(at))
                            .cloned(),
                    ),
                    _ => None,
                };
                match chosen {
                    Some(link) => {
                        open = link.map(|(_, address)| address);
                        self.listing = None;
                        continue;
                    }
                    None if !matches!(reading, Reading::Scroll(_) | Reading::Page(_)) => {
                        self.listing = None;
                    }
                    None => {}
                }
            }
            if let Some((number, first)) = self.zoom {
                let page = height.saturating_sub(1).max(1) as isize;
                let by = match reading {
                    Reading::Scroll(rows) => Some(*rows),
                    Reading::Page(pages) => Some(pages * page),
                    _ => None,
                };
                match by {
                    Some(by) => {
                        let rows = laid
                            .enlarged(laid.at(number), &mut Vec::new())
                            .map_or(0, |lines| lines.len());
                        let most = rows.saturating_sub(height);
                        let first = (first as isize + by).clamp(0, most as isize);
                        self.zoom = Some((number, first as usize));
                    }
                    None => self.zoom = None,
                }
                if matches!(
                    reading,
                    Reading::Scroll(_) | Reading::Page(_) | Reading::Click(..) | Reading::Toggle
                ) {
                    continue;
                }
            }
            match reading {
                Reading::Scroll(rows) => self.scroll(&laid, height, *rows),
                Reading::Page(pages) => {
                    let rows = height.saturating_sub(1).max(1) as isize;
                    self.scroll(&laid, height, pages * rows);
                }
                Reading::Mark(by) => self.move_mark(&laid, height, *by),
                Reading::Toggle => match self.mark {
                    Some(Spot::Block(number)) => self.open(&laid, height, number),
                    Some(Spot::Back(number)) => {
                        self.go_to(&laid, height, laid.head_of(laid.at(number)));
                        self.mark = None;
                    }
                    None => {}
                },
                Reading::Click(column, row) => {
                    let shown = usize::from(row.saturating_sub(self.from));
                    let clicked = (*row >= self.from)
                        .then(|| self.rows.get(shown).copied().flatten())
                        .flatten();
                    // On a link, it opens: what holds it is only marked.
                    if let Some(link) = self.link_at(*column, *row) {
                        open = Some(link.address.clone());
                        if clicked.is_some() {
                            self.mark = clicked;
                        }
                        continue;
                    }
                    match clicked {
                        Some(Spot::Block(number)) => {
                            self.mark = clicked;
                            self.open(&laid, height, number);
                        }
                        Some(Spot::Back(number)) => {
                            self.go_to(&laid, height, laid.head_of(laid.at(number)));
                            self.mark = None;
                        }
                        None => {
                            self.mark = None;
                            missed = true;
                        }
                    }
                }
                Reading::Find(words) => {
                    let found = laid.holding(words);
                    said = Some(match found.len() {
                        0 => format!("nothing here holds “{words}”"),
                        1 => format!("“{words}” found once"),
                        count => format!("“{words}” found {count} times: n, N go through them"),
                    });
                    self.mark = found.last().copied().map(Spot::Block);
                    if let Some(number) = found.last() {
                        self.show(&laid, height, laid.at(*number));
                    }
                    self.found = Some((words.clone(), found));
                }
                Reading::Match(by) => {
                    if let Some((_, found)) = &self.found
                        && !found.is_empty()
                    {
                        let now = self
                            .mark
                            .and_then(Spot::block)
                            .and_then(|mark| found.iter().position(|n| *n == mark));
                        let at = match now {
                            Some(now) => now.saturating_add_signed(*by).min(found.len() - 1),
                            None => found.len() - 1,
                        };
                        let number = found[at];
                        self.mark = Some(Spot::Block(number));
                        self.show(&laid, height, laid.at(number));
                    }
                }
                Reading::Copy => {
                    if let Some(text) = self
                        .mark
                        .and_then(Spot::block)
                        .and_then(|mark| laid.copied(laid.at(mark)))
                    {
                        let lines = text.lines().count().max(1);
                        said = Some(format!(
                            "copied {lines} line{}",
                            if lines == 1 { "" } else { "s" }
                        ));
                        copied = Some(text);
                    }
                }
                Reading::Open | Reading::Pick(_) => {
                    let links = self
                        .mark
                        .and_then(Spot::block)
                        .map(|mark| laid.links(laid.at(mark)))
                        .unwrap_or_default();
                    match (reading, links.len()) {
                        (_, 0) => said = Some("nothing marked holds a link".to_owned()),
                        (Reading::Pick(number), count) => {
                            match number.checked_sub(1).and_then(|at| links.get(at)) {
                                Some((_, address)) => open = Some(address.clone()),
                                None => {
                                    said = Some(format!(
                                        "what is marked holds {count} link{}",
                                        if count == 1 { "" } else { "s" }
                                    ));
                                }
                            }
                        }
                        (_, 1) => open = links.into_iter().next().map(|(_, address)| address),
                        _ => self.listing = Some(Listing { links, picked: 0 }),
                    }
                }
                Reading::Top | Reading::End | Reading::Answer(_) => {
                    self.answer(&laid, height, reading);
                    self.mark = None;
                }
                Reading::Leave => {
                    self.mark = None;
                    self.found = None;
                }
            }
        }
        let unfolded = self.unfolded.clone();
        let laid = Laid {
            opening: history.opening_lines(area.width),
            first,
            blocks,
            width: area.width,
            height,
            cell: history.cell(),
            board,
            p,
            unfolded: &unfolded,
            counted: RefCell::default(),
        };
        let mark = self.mark;
        let enlarged = self.zoom.and_then(|(number, first)| {
            let mut images = Vec::new();
            let lines = laid.enlarged(laid.at(number), &mut images)?;
            Some((number, first, lines, images))
        });
        let mut shown = match enlarged {
            Some((number, first, lines, images)) => {
                let lines: Vec<Line<'static>> =
                    lines.into_iter().skip(first).take(height).collect();
                self.rows = vec![Some(Spot::Block(number)); lines.len()];
                Shown {
                    lines,
                    images,
                    ..Shown::default()
                }
            }
            None => {
                self.zoom = None;
                self.lay_out(&laid, height)
            }
        };
        shown.missed = missed;
        shown.copied = copied;
        shown.said = said;
        shown.open = open;
        // A link's address rides on its spans' background: taken off them,
        // kept for where each shows.
        self.links = shown.lines.iter_mut().map(unlinked).collect();
        if let Some((words, _)) = &self.found {
            for line in &mut shown.lines {
                highlighted(line, words);
            }
        }
        self.from = area.from;
        if let Some(mark) = mark {
            for (line, number) in shown.lines.iter_mut().zip(&self.rows) {
                if *number == Some(mark) {
                    marked(line, p);
                }
            }
        }
        if let Some(listing) = self.listing.take() {
            self.list(&mut shown, &listing, area, p);
            self.listing = Some(listing);
        }
        shown.pointed = self
            .pointer
            .and_then(|(column, row)| self.link_at(column, row))
            .map(|link| Pointed {
                address: link.address.clone(),
                more: 0,
                mouse: true,
            })
            .or_else(|| {
                let (address, more) = match &self.listing {
                    Some(listing) => (listing.links.get(listing.picked)?.1.clone(), 0),
                    None => {
                        let links = laid.links(laid.at(mark?.block()?));
                        let more = links.len().saturating_sub(1);
                        (links.into_iter().next()?.1, more)
                    }
                };
                Some(Pointed {
                    address,
                    more,
                    mouse: false,
                })
            });
        shown
    }

    /// The mouse is now over the cell at `column` and `row` of the screen.
    /// Returns whether it points at another link than it did.
    pub(crate) fn point(&mut self, column: u16, row: u16) -> bool {
        let at = |view: &View, (column, row): (u16, u16)| {
            view.link_at(column, row).map(|link| (row, link.from))
        };
        let before = self.pointer.and_then(|pointer| at(self, pointer));
        self.pointer = Some((column, row));
        before != at(self, (column, row))
    }

    /// The link the cell at `column` and `row` of the screen showed, as
    /// last drawn.
    fn link_at(&self, column: u16, row: u16) -> Option<&Link> {
        let column = usize::from(column);
        self.links
            .get(usize::from(row.checked_sub(self.from)?))?
            .iter()
            .find(|link| (link.from..link.to).contains(&column))
    }

    /// The links listed, over the last rows of the conversation, each a
    /// row a click opens.
    fn list(&mut self, shown: &mut Shown, listing: &Listing, area: Area, p: &Palette) {
        let height = area.height.max(1);
        let room = height.saturating_sub(1).max(1);
        let first = listing.picked.saturating_sub(room - 1);
        let mut lines = vec![Line::from(Span::styled(
            "  links in what is marked · ↑ ↓ and Enter, its number or a click opens one · Esc",
            p.fg(p.faint),
        ))];
        let mut links = vec![Vec::new()];
        for (at, (text, address)) in listing.links.iter().enumerate().skip(first).take(room) {
            let picked = at == listing.picked;
            let lead = if picked { "› " } else { "  " };
            lines.push(crate::wrap::fit(
                vec![
                    Span::styled(lead, p.bold(p.accent)),
                    Span::styled(format!("{} ", at + 1), p.fg(p.dim)),
                    Span::styled(
                        text.clone(),
                        if picked { p.bold(p.text) } else { p.fg(p.text) },
                    ),
                    Span::raw("  "),
                    Span::styled(
                        address.clone(),
                        p.fg(p.info).add_modifier(Modifier::UNDERLINED),
                    ),
                ],
                area.width,
            ));
            links.push(vec![Link {
                from: 0,
                to: area.width,
                number: at + 1,
                address: address.clone(),
            }]);
        }
        let over = lines.len().min(height);
        shown.lines.resize(height, Line::default());
        self.rows.resize(height, None);
        self.links.resize(height, Vec::new());
        let start = height - over;
        for (row, (line, links)) in lines.into_iter().zip(links).take(over).enumerate() {
            shown.lines[start + row] = line;
            self.rows[start + row] = None;
            self.links[start + row] = links;
        }
    }

    /// Opens the block numbered `number`: a diagram enlarged, what is
    /// folded unfolded, or folded again.
    fn open(&mut self, laid: &Laid<'_>, height: usize, number: usize) {
        let at = laid.at(number);
        if laid.foldable(at) {
            self.toggle(laid, height, number);
        } else if laid.enlarged(at, &mut Vec::new()).is_some() {
            self.zoom = Some((number, 0));
        }
    }

    /// Unfolds the block numbered `number`, or folds it again: what is
    /// above it stays where it is, what follows it moves.
    fn toggle(&mut self, laid: &Laid<'_>, height: usize, number: usize) {
        let (at, row) = self.top_now(laid, height);
        self.top = Some((laid.number(at), row));
        if !self.unfolded.remove(&number) {
            self.unfolded.insert(number);
        }
    }

    /// Moves the mark to what is folded, a diagram or the mark after an
    /// answer, before it or after it, by `by`; from what shows when none
    /// is marked, up from its last row or down from its first. Scrolls to
    /// show it.
    fn move_mark(&mut self, laid: &Laid<'_>, height: usize, by: isize) {
        let stops = |at: usize| -> Vec<Spot> {
            let number = laid.number(at);
            let mut stops = Vec::new();
            if laid.markable(at) {
                stops.push(Spot::Block(number));
            }
            if laid.ends_tall(at) {
                stops.push(Spot::Back(number));
            }
            stops
        };
        let shown = |last: bool| {
            let mut spots = self.rows.iter().flatten();
            let spot = if last { spots.last() } else { spots.next() };
            spot.map_or(if last { laid.len() - 1 } else { 0 }, |spot| {
                laid.at(spot.number())
            })
        };
        // Where it goes from: a place, and where in it.
        let (from, rank) = match self.mark {
            Some(spot) => (laid.at(spot.number()), spot.rank()),
            None if by < 0 => (shown(true), 3),
            None => (shown(false), 0),
        };
        let found = if by < 0 {
            (0..=from).rev().find_map(|at| {
                stops(at)
                    .into_iter()
                    .rev()
                    .find(|spot| at < from || spot.rank() < rank)
                    .map(|spot| (at, spot))
            })
        } else {
            (from..laid.len()).find_map(|at| {
                stops(at)
                    .into_iter()
                    .find(|spot| at > from || spot.rank() > rank)
                    .map(|spot| (at, spot))
            })
        };
        if let Some((at, spot)) = found {
            self.mark = Some(spot);
            self.show(laid, height, at);
        }
    }

    /// Goes to the top of the answer the view's top is in, to its end, or
    /// to the top of the answer before or after it, as `reading` asks.
    fn answer(&mut self, laid: &Laid<'_>, height: usize, reading: &Reading) {
        let (mut top, _) = self.top_now(laid, height);
        // A line left empty before a turn is read with it.
        if laid.blank(top) && top + 1 < laid.len() {
            top += 1;
        }
        let here = laid.head_of(top);
        match reading {
            Reading::Top => self.go_to(laid, height, here),
            Reading::End => {
                // What the person wrote is read with the answer after it.
                let mut end = laid.end_of(top);
                if laid.head(here) == Some(Speaker::Person)
                    && let Some(next) = (end + 1..laid.len()).find(|at| laid.head(*at).is_some())
                    && laid.head(next) == Some(Speaker::Agent)
                {
                    end = laid.end_of(next);
                }
                let by = self.offset(laid, height, end) + laid.rows(end) as isize - height as isize;
                self.scroll(laid, height, by.max(0));
            }
            // Its own top first, when it is above the view.
            Reading::Answer(by) if *by < 0 => {
                let to = if self.offset(laid, height, here) < 0 {
                    here
                } else {
                    laid.head_of(here.saturating_sub(1))
                };
                self.go_to(laid, height, to);
            }
            _ => match (top + 1..laid.len()).find(|at| laid.head(*at).is_some()) {
                Some(next) => self.go_to(laid, height, next),
                None => self.top = None,
            },
        }
    }

    /// Scrolls to put place `at` at the view's top, as far as the end lets.
    fn go_to(&mut self, laid: &Laid<'_>, height: usize, at: usize) {
        let by = self.offset(laid, height, at);
        self.scroll(laid, height, by);
    }

    /// Where place `at` begins, in rows from the view's top: fewer than
    /// none above it.
    fn offset(&self, laid: &Laid<'_>, height: usize, at: usize) -> isize {
        let (top, row) = self.top_now(laid, height);
        if at >= top {
            (top..at)
                .map(|place| laid.rows(place) as isize)
                .sum::<isize>()
                - row as isize
        } else {
            -((at..top)
                .map(|place| laid.rows(place) as isize)
                .sum::<isize>()
                + row as isize)
        }
    }

    /// Scrolls to show place `at`: whole when it fits, its start at least.
    fn show(&mut self, laid: &Laid<'_>, height: usize, at: usize) {
        let start = self.offset(laid, height, at);
        let end = start + laid.rows(at) as isize;
        if start < 0 {
            self.scroll(laid, height, start);
        } else if end > height as isize {
            self.scroll(laid, height, (end - height as isize).min(start));
        }
    }

    /// The rows from the view's top, or the last `height` while it
    /// follows the end: it follows it again once that is reached. Notes
    /// which block each row shows.
    fn lay_out(&mut self, laid: &Laid<'_>, height: usize) -> Shown {
        let mut shown = Shown::default();
        let mut rows = Vec::new();
        if let Some((number, row)) = self.top {
            let mut at = laid.at(number);
            let mut skip = row;
            while at < laid.len() && shown.lines.len() < height {
                let mut images = Vec::new();
                let lines = laid.lines(at, &mut images);
                let spots = laid.spots(at, lines.len());
                let taken: Vec<Line<'static>> = lines.into_iter().skip(skip).collect();
                if !taken.is_empty() {
                    shown.images.extend(images);
                }
                rows.extend(spots.into_iter().skip(skip));
                shown.lines.extend(taken);
                skip = 0;
                at += 1;
            }
            if shown.lines.len() >= height {
                shown.lines.truncate(height);
                rows.truncate(height);
                self.rows = rows;
                return shown;
            }
            self.top = None;
            shown = Shown::default();
            rows.clear();
        }
        // From the end up, as many rows as there is room for.
        let mut parts = Vec::new();
        let mut count = 0;
        for at in (0..laid.len()).rev() {
            if count >= height {
                break;
            }
            let mut images = Vec::new();
            let lines = laid.lines(at, &mut images);
            count += lines.len();
            parts.push((at, lines, images));
        }
        for (at, lines, images) in parts.into_iter().rev() {
            rows.extend(laid.spots(at, lines.len()));
            shown.lines.extend(lines);
            shown.images.extend(images);
        }
        let over = shown.lines.len().saturating_sub(height);
        shown.lines.drain(..over);
        rows.drain(..over);
        self.rows = rows;
        shown
    }

    /// The view's top, the place and the row in it, as it now stands.
    fn top_now(&self, laid: &Laid<'_>, height: usize) -> (usize, usize) {
        if let Some((number, row)) = self.top {
            return (laid.at(number), row);
        }
        let mut above = height;
        for at in (0..laid.len()).rev() {
            let rows = laid.rows(at);
            if rows >= above {
                return (at, rows - above);
            }
            above -= rows;
        }
        (0, 0)
    }

    /// Moves the view by `rows`, up when fewer than none, never above the
    /// start; back down to the end, it follows it again.
    fn scroll(&mut self, laid: &Laid<'_>, height: usize, rows: isize) {
        let (mut at, mut row) = self.top_now(laid, height);
        if rows < 0 {
            let mut up = rows.unsigned_abs();
            while up > row {
                if at == 0 {
                    up = row;
                    break;
                }
                up -= row + 1;
                at -= 1;
                row = laid.rows(at).saturating_sub(1);
            }
            row -= up;
        } else {
            let mut down = rows.unsigned_abs();
            loop {
                let left = laid.rows(at).saturating_sub(row);
                if down < left || at + 1 >= laid.len() {
                    row = (row + down).min(laid.rows(at).saturating_sub(1));
                    break;
                }
                down -= left;
                at += 1;
                row = 0;
            }
        }
        // What lies below the top fills the view no more: the end.
        let mut below = laid.rows(at).saturating_sub(row);
        let mut next = at + 1;
        while below <= height && next < laid.len() {
            below += laid.rows(next);
            next += 1;
        }
        self.top = (below > height).then(|| (laid.number(at), row));
    }
}

/// The links `line` shows, by their columns, their addresses taken off
/// their spans' background, which the terminal would paint.
fn unlinked(line: &mut Line<'static>) -> Vec<Link> {
    let mut links: Vec<Link> = Vec::new();
    let mut column = 0;
    for span in &mut line.spans {
        let width = span.width();
        if let Some((number, address, plain)) = crate::link::address_of(span.style) {
            span.style = plain;
            match links.last_mut() {
                Some(link) if link.number == number && link.to == column => link.to += width,
                _ => links.push(Link {
                    from: column,
                    to: column + width,
                    number,
                    address,
                }),
            }
        }
        column += width;
    }
    links
}

/// `line` with `words` shown reversed wherever it holds them, whatever
/// their case.
fn highlighted(line: &mut Line<'static>, words: &str) {
    let words = words.to_lowercase();
    if words.is_empty() || !line.to_string().to_lowercase().contains(&words) {
        return;
    }
    let mut spans = Vec::new();
    for span in std::mem::take(&mut line.spans) {
        let text = span.content.to_string();
        let lower = text.to_lowercase();
        // Lowercase may change lengths: only where it keeps them.
        if lower.len() != text.len() || !lower.contains(&words) {
            spans.push(span);
            continue;
        }
        let mut from = 0;
        for (at, _) in lower.match_indices(&words) {
            if at > from {
                spans.push(Span::styled(text[from..at].to_owned(), span.style));
            }
            spans.push(Span::styled(
                text[at..at + words.len()].to_owned(),
                span.style.add_modifier(ratatui::style::Modifier::REVERSED),
            ));
            from = at + words.len();
        }
        if from < text.len() {
            spans.push(Span::styled(text[from..].to_owned(), span.style));
        }
    }
    line.spans = spans;
}

/// `line` marked: a bar where it begins, in the accent's colour, over the
/// space its indent leaves there.
fn marked(line: &mut Line<'static>, p: &Palette) {
    let Some(first) = line.spans.iter().position(|span| !span.content.is_empty()) else {
        line.spans.push(Span::styled("▌", p.bold(p.accent)));
        return;
    };
    let span = &line.spans[first];
    if let Some(rest) = span.content.strip_prefix(' ') {
        let rest = Span::styled(rest.to_owned(), span.style);
        line.spans
            .splice(first..=first, [Span::styled("▌", p.bold(p.accent)), rest]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Depth;
    use iq_agent::{Event, PERSON};
    use iq_ui::Update;
    use iq_ui::look::Theme;

    fn crew(board: &mut Board, history: &mut History, p: &Palette, event: Event) {
        board.update(Update::Crew(event.clone()));
        history.event(&event, board, p);
    }

    fn said(board: &mut Board, history: &mut History, p: &Palette, text: &str) {
        let event = Event::Text {
            agent: "lead".into(),
            text: text.into(),
        };
        crew(board, history, p, event);
    }

    fn rows(shown: &Shown) -> Vec<String> {
        shown
            .lines
            .iter()
            .map(|l| l.to_string().trim_end().to_owned())
            .collect()
    }

    /// A board and a history with the chat `lead` joined, and its window
    /// taken.
    fn joined() -> (Palette, Board, History) {
        let p = Palette::new(Theme::Graphite, Depth::True);
        let mut board = Board::default();
        let mut history = History::default();
        crew(
            &mut board,
            &mut history,
            &p,
            Event::Joined {
                agent: "lead".into(),
                model: "lab/lead".into(),
                by: PERSON.into(),
            },
        );
        (p, board, history)
    }

    fn area(height: usize) -> Area {
        Area {
            from: 2,
            width: 60,
            height,
        }
    }

    #[test]
    fn the_view_follows_the_end_and_stays_put_once_scrolled_up() {
        let (p, mut board, mut history) = joined();
        let lines: String = (1..=20).map(|n| format!("line {n}\n\n")).collect();
        said(&mut board, &mut history, &p, &lines);
        let _ = history.take(60, Some("lead"));
        let mut view = View::default();
        let mut shown = |history: &History, board: &Board, readings: &[Reading]| {
            rows(&view.shown(history, board, &p, Some("lead"), area(5), readings))
        };
        // Taller than the room, the answer ends with the way back to its
        // top.
        let end = shown(&history, &board, &[]);
        assert_eq!(
            end[end.len() - 2..],
            ["  line 20", "  ↑ top of this answer"]
        );
        // Up by 3 pages less a row each: what shows stays as more comes.
        let before = shown(&history, &board, &[Reading::Page(-3)]);
        assert!(before.contains(&"  line 14".to_owned()), "{before:#?}");
        said(&mut board, &mut history, &p, "line 21\n");
        let _ = history.take(60, Some("lead"));
        assert_eq!(shown(&history, &board, &[]), before);
        // Never above the start.
        let top = shown(&history, &board, &[Reading::Scroll(-500)]);
        assert_eq!(top[1], "● lead");
        // Back down to the end, it follows it again.
        let end = shown(&history, &board, &[Reading::Scroll(500)]);
        assert_eq!(end[end.len() - 2], "  line 21");
    }

    #[test]
    fn answers_are_read_one_after_another_and_a_long_one_goes_back_to_its_top() {
        let (p, mut board, mut history) = joined();
        let tall: String = (1..=12).map(|n| format!("line {n}\n\n")).collect();
        for (question, answer) in [("first?", "short\n"), ("second?", tall.as_str())] {
            // A later turn is the person's message.
            crew(
                &mut board,
                &mut history,
                &p,
                Event::Posted {
                    from: PERSON.into(),
                    to: "lead".into(),
                    text: question.into(),
                },
            );
            for event in [
                Event::Working {
                    agent: "lead".into(),
                    prompt: question.into(),
                },
                Event::Text {
                    agent: "lead".into(),
                    text: answer.into(),
                },
                Event::Done {
                    agent: "lead".into(),
                    outcome: Ok("done".into()),
                },
            ] {
                crew(&mut board, &mut history, &p, event);
            }
        }
        let _ = history.take(60, Some("lead"));
        let mut view = View::default();
        let mut shown = |readings: &[Reading]| {
            rows(&view.shown(&history, &board, &p, Some("lead"), area(8), readings))
        };
        // Taller than the room: a click on the mark after it goes back to
        // its top.
        let end = shown(&[]);
        assert_eq!(
            end.last().map(String::as_str),
            Some("  ↑ top of this answer")
        );
        let back = Reading::Click(4, 2 + 7);
        assert_eq!(shown(&[back])[0], "● lead");
        // Shift and the arrows go from one answer to the next, Home and
        // End to the top and the end of the one read.
        assert_eq!(shown(&[Reading::Answer(-1)])[0], "› second?");
        assert_eq!(shown(&[Reading::Answer(-1)])[0], "● lead");
        assert_eq!(shown(&[Reading::Answer(-1)])[0], "› first?");
        assert_eq!(shown(&[Reading::Answer(1)])[0], "● lead");
        assert_eq!(shown(&[Reading::Answer(1)])[0], "› second?");
        assert_eq!(shown(&[Reading::Answer(1)])[0], "● lead");
        let end = shown(&[Reading::End]);
        assert_eq!(
            end.last().map(String::as_str),
            Some("  ↑ top of this answer")
        );
        assert_eq!(shown(&[Reading::Top])[0], "● lead");
        // From what the person wrote, the end of the answer after it.
        assert_eq!(shown(&[Reading::Answer(-1)])[0], "› second?");
        assert_eq!(shown(&[Reading::End]), end);
        // Past the last, the end, followed again.
        assert_eq!(shown(&[Reading::Answer(1)]), end);
        // The mark goes to it from what shows, and Enter goes back up.
        let marked = shown(&[Reading::Mark(-1)]);
        assert_eq!(
            marked.last().map(String::as_str),
            Some("▌ ↑ top of this answer")
        );
        assert_eq!(shown(&[Reading::Toggle])[0], "● lead");
    }

    /// Draws every diagram at once as a PNG of 400 by 600 pixels.
    struct Tall;

    impl crate::diagram::Draw for Tall {
        fn draw(&self, _source: &str) -> Result<crate::diagram::Drawn, String> {
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
            png.extend(400u32.to_be_bytes());
            png.extend(600u32.to_be_bytes());
            Ok(crate::diagram::Drawn { png })
        }
    }

    #[test]
    fn a_click_enlarges_a_diagram_which_takes_the_wheel_until_put_back() {
        let (p, mut board, mut history) = joined();
        history.pictures(Some(std::sync::Arc::new(Tall)), (10, 20));
        said(
            &mut board,
            &mut history,
            &p,
            "```mermaid\ngraph TD\n A --> B\n```\nAfter it.\n",
        );
        for _ in 0..500 {
            let _ = history.take(60, Some("lead"));
            if !history.waiting() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut view = View::default();
        let placed = |shown: &Shown| {
            shown
                .lines
                .iter()
                .filter(|line| line.to_string().contains('\u{10EEEE}'))
                .count()
        };
        let small = view.shown(&history, &board, &p, Some("lead"), area(20), &[]);
        let row = small
            .lines
            .iter()
            .position(|line| line.to_string().contains('\u{10EEEE}'))
            .unwrap();
        let click = Reading::Click(10, 2 + row as u16);
        let large = view.shown(&history, &board, &p, Some("lead"), area(20), &[click]);
        // Alone, the conversation's whole room, wider than it was shown.
        assert_eq!(placed(&large), 20);
        assert!(large.images[0].cols > small.images[0].cols);
        assert!(!rows(&large).iter().any(|row| row.contains("After it.")));
        // The wheel moves through it, never past its end.
        let first = large.lines[0].to_string();
        let moved = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Scroll(3)],
        );
        assert_ne!(moved.lines[0].to_string(), first);
        assert_eq!(placed(&moved), 20);
        // Clicked again, it is back in the conversation.
        let back = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Click(10, 5)],
        );
        assert!(rows(&back).iter().any(|row| row.contains("After it.")));
    }

    #[test]
    fn words_found_are_marked_in_turn_and_what_is_marked_copied_as_written() {
        let (p, mut board, mut history) = joined();
        said(
            &mut board,
            &mut history,
            &p,
            "The Parser reads it.\n\n```rust\nfn parse() {}\nfn check() {}\n```\n\nThen the parser checks it.\n",
        );
        let _ = history.take(60, Some("lead"));
        let mut view = View::default();
        let find = Reading::Find("parser".into());
        let found = view.shown(&history, &board, &p, Some("lead"), area(20), &[find]);
        assert_eq!(
            found.said.as_deref(),
            Some("“parser” found 2 times: n, N go through them")
        );
        // The newest marked, each match shown reversed.
        let rows = rows(&found);
        assert!(
            rows.iter().any(|row| row == "▌ Then the parser checks it."),
            "{rows:#?}"
        );
        let reversed = |line: &Line<'_>| {
            line.spans.iter().any(|span| {
                span.content == "Parser"
                    && span
                        .style
                        .add_modifier
                        .contains(ratatui::style::Modifier::REVERSED)
            })
        };
        assert!(found.lines.iter().any(reversed));
        // The one before, then copied as written.
        let older = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Match(-1), Reading::Copy],
        );
        assert_eq!(older.copied.as_deref(), Some("The Parser reads it."));
        assert_eq!(older.said.as_deref(), Some("copied 1 line"));
        // A block of code copied whole, as it is.
        let code = older
            .lines
            .iter()
            .position(|line| line.to_string().contains("fn check"))
            .unwrap();
        let copied = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Click(3, 2 + code as u16), Reading::Copy],
        );
        assert_eq!(
            copied.copied.as_deref(),
            Some("fn parse() {}\nfn check() {}")
        );
        // Done reading, nothing is marked nor shown reversed.
        let left = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Leave],
        );
        assert!(!left.lines.iter().any(reversed));
    }

    #[test]
    fn a_link_opens_with_a_click_or_a_key_its_address_said_first() {
        let (p, mut board, mut history) = joined();
        history.set_links(true);
        said(
            &mut board,
            &mut history,
            &p,
            "See [the docs](https://example.com/docs) first.\n\nThen [one](https://example.com/1) and [two](https://example.com/2).\n",
        );
        let _ = history.take(60, Some("lead"));
        let mut view = View::default();
        let first = view.shown(&history, &board, &p, Some("lead"), area(20), &[]);
        // Underlined, its address off the background the terminal paints.
        let row = first
            .lines
            .iter()
            .position(|line| line.to_string().contains("the docs"))
            .unwrap();
        let docs = first.lines[row]
            .spans
            .iter()
            .find(|span| span.content == "the docs")
            .unwrap();
        assert!(docs.style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(docs.style.bg, None);
        let column = first.lines[row].to_string().find("the docs").unwrap() as u16;
        // The mouse over it, the status line says where it leads.
        assert!(view.point(column + 2, 2 + row as u16));
        assert!(!view.point(column + 3, 2 + row as u16));
        let over = view.shown(&history, &board, &p, Some("lead"), area(20), &[]);
        assert_eq!(
            over.pointed.as_ref().map(|p| (p.address.as_str(), p.mouse)),
            Some(("https://example.com/docs", true))
        );
        // Clicked, it opens, and nothing else happens.
        let click = Reading::Click(column + 1, 2 + row as u16);
        let opened = view.shown(&history, &board, &p, Some("lead"), area(20), &[click]);
        assert_eq!(opened.open.as_deref(), Some("https://example.com/docs"));
        assert_eq!(rows(&opened)[row], "▌ See the docs first.");
        assert_eq!(rows(&opened).len(), rows(&first).len());
        // Marked, what holds two lists them, to choose one.
        assert!(view.point(0, 0));
        let both = opened
            .lines
            .iter()
            .position(|line| line.to_string().contains("Then"))
            .unwrap();
        let readings = [Reading::Click(0, 2 + both as u16), Reading::Open];
        let listed = view.shown(&history, &board, &p, Some("lead"), area(20), &readings);
        assert_eq!(listed.open, None);
        let listed = rows(&listed);
        assert!(
            listed.contains(&"› 1 one  https://example.com/1".to_owned()),
            "{listed:#?}"
        );
        assert!(listed.contains(&"  2 two  https://example.com/2".to_owned()));
        let chosen = [Reading::Mark(1), Reading::Toggle];
        let opened = view.shown(&history, &board, &p, Some("lead"), area(20), &chosen);
        assert_eq!(opened.open.as_deref(), Some("https://example.com/2"));
        assert!(!rows(&opened).iter().any(|row| row.contains("links in")));
        // Or by its number, the first said with how many more there are.
        let pointed = view.shown(&history, &board, &p, Some("lead"), area(20), &[]);
        let pointed = pointed.pointed.unwrap();
        assert_eq!(
            (pointed.address.as_str(), pointed.more),
            ("https://example.com/1", 1)
        );
        let picked = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Pick(2)],
        );
        assert_eq!(picked.open.as_deref(), Some("https://example.com/2"));
    }

    #[test]
    fn a_click_unfolds_a_fold_in_place_alone_and_folds_it_again() {
        let (p, mut board, mut history) = joined();
        let read = |path: &str| Event::Tool {
            agent: "lead".into(),
            name: "read_file".into(),
            input: path.into(),
            output: format!("1\tin {path}"),
            ok: true,
        };
        for event in [
            Event::Working {
                agent: "lead".into(),
                prompt: "look".into(),
            },
            read("src/a.rs"),
            read("src/b.rs"),
            Event::TextStart {
                agent: "lead".into(),
            },
            Event::Text {
                agent: "lead".into(),
                text: "Read.\n".into(),
            },
            read("src/c.rs"),
            read("src/d.rs"),
            Event::Done {
                agent: "lead".into(),
                outcome: Ok("done".into()),
            },
        ] {
            crew(&mut board, &mut history, &p, event);
        }
        let _ = history.take(60, Some("lead"));
        let mut view = View::default();
        let folded = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &[]));
        let first = folded
            .iter()
            .position(|row| row.contains("2 steps: read_file ×2") && row.contains("#1"))
            .unwrap();
        // Clicked, the first fold unfolds under its count, alone: what is
        // above it stays.
        let click = Reading::Click(5, 2 + first as u16);
        let unfolded = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &[click]));
        assert_eq!(unfolded[..first], folded[..first]);
        assert!(unfolded[first].starts_with("▌ ▸ 2 steps"), "{unfolded:#?}");
        // Marked, a bar where its rows begin.
        assert_eq!(unfolded[first + 1], "▌ ▸ read_file src/a.rs");
        assert_eq!(unfolded[first + 2], "▌   1\tin src/a.rs");
        assert!(
            !unfolded.iter().any(|row| row.contains("src/c.rs")),
            "{unfolded:#?}"
        );
        // Clicked again, it folds.
        let click = Reading::Click(5, 2 + first as u16);
        let again = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &[click]));
        assert_eq!(again[first + 1], folded[first + 1]);
        // With the keys: the last fold marked, unfolded, then the one before.
        let keys = [Reading::Leave, Reading::Mark(-1), Reading::Toggle];
        let last = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &keys));
        assert!(
            last.iter().any(|row| row == "▌ ▸ read_file src/c.rs"),
            "{last:#?}"
        );
        let keys = [Reading::Mark(-1), Reading::Toggle];
        let both = rows(&view.shown(&history, &board, &p, Some("lead"), area(20), &keys));
        assert!(
            both.iter().any(|row| row == "▌ ▸ read_file src/a.rs"),
            "{both:#?}"
        );
        assert!(
            both.iter().any(|row| row == "  ▸ read_file src/c.rs"),
            "{both:#?}"
        );
        // A click below the conversation misses it.
        let missed = view.shown(
            &history,
            &board,
            &p,
            Some("lead"),
            area(20),
            &[Reading::Click(0, 40)],
        );
        assert!(missed.missed);
    }
}
