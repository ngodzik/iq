//! The Mermaid diagrams agents write, drawn as images once whole, in Kitty,
//! by what the program gives the terminal to draw them with ([`Draw`]), as
//! the settings let it; elsewhere, or when nothing draws them, written as
//! their source. A diagram being drawn holds back what its window writes
//! after it, so that the conversation keeps its order.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ratatui::text::{Line, Span};

use crate::graphics;
use crate::wrap::cut;

/// What draws the diagrams as images, as the settings choose; it does the
/// I/O.
pub trait Draw: Send + Sync {
    /// Draws the Mermaid diagram `source` as a PNG image; waits for it.
    ///
    /// # Errors
    ///
    /// Why it could not be drawn.
    fn draw(&self, source: &str) -> Result<Drawn, String>;
}

/// A diagram drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    /// The image, a PNG.
    pub png: Vec<u8>,
}

/// The most rows an image takes, so that one fits on screen with what was
/// said of it.
const MOST_ROWS: u16 = 30;

/// How much larger than its pixels a diagram is shown, as a fraction: half
/// as large again, so that it reads on dense cells.
const LARGER: (u32, u32) = (3, 2);

/// How many times wider than tall a diagram is that takes the whole width.
const WIDE: u32 = 3;

/// How long a diagram is waited for before it is written as its source.
const PATIENCE: Duration = Duration::from_secs(90);

/// Where a diagram stands.
#[derive(Debug)]
enum State {
    Drawing(Instant),
    Drawn(Drawn),
    Failed(String),
    /// Nothing draws it: its source is written, as any code.
    Source,
}

/// An image to send to the terminal before the lines it shows under.
#[derive(Debug, Clone)]
pub(crate) struct Image {
    pub(crate) id: u32,
    pub(crate) png: Arc<Vec<u8>>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// What a diagram shows where it is written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Shown {
    /// Its image's cells.
    Image(Vec<Line<'static>>),
    /// Its source, as code, with why it was not drawn when it failed.
    Source(Option<String>),
}

/// A diagram of the conversation: where its drawing stands, and the ids
/// its image has with the terminal, one for each size it is shown at.
#[derive(Debug)]
pub(crate) struct Diagram {
    /// Where the ids of images come from.
    ids: Arc<AtomicU32>,
    /// The id of each size it was shown at, in cells: the terminal keeps
    /// an image with the cells it shows under, so that an image shown at
    /// another size, as the screen was made narrower, is another, and
    /// what the scrollback shows of the first stays as it was.
    shown: Mutex<Vec<((u16, u16), u32)>>,
    state: Mutex<State>,
}

/// The next id an image takes with the terminal, from `ids`: 24 bits, as a
/// placeholder's colour carries it, and never 0.
pub(crate) fn next_id(ids: &AtomicU32) -> u32 {
    loop {
        let id = ids.fetch_add(1, Ordering::Relaxed) & 0x00ff_ffff;
        if id != 0 {
            return id;
        }
    }
}

impl Diagram {
    /// The diagram of `source`, which `draw` draws on a thread of its own
    /// when given, its images taking their ids from `ids`.
    pub(crate) fn new(
        source: String,
        ids: Arc<AtomicU32>,
        draw: Option<Arc<dyn Draw>>,
    ) -> Arc<Diagram> {
        let diagram = Arc::new(Diagram {
            ids,
            shown: Mutex::new(Vec::new()),
            state: Mutex::new(if draw.is_some() {
                State::Drawing(Instant::now())
            } else {
                State::Source
            }),
        });
        if let Some(draw) = draw {
            let shared = Arc::clone(&diagram);
            std::thread::spawn(move || {
                let state = match draw.draw(&source) {
                    Ok(drawn) if graphics::png_size(&drawn.png).is_some() => State::Drawn(drawn),
                    Ok(_) => State::Failed("what drew it is no PNG image".to_owned()),
                    Err(why) => State::Failed(why),
                };
                *shared.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
            });
        }
        diagram
    }

    /// The id of its image shown on `cells`, columns by rows.
    fn id(&self, cells: (u16, u16)) -> u32 {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, id)) = shown.iter().find(|(seen, _)| *seen == cells) {
            return *id;
        }
        let id = next_id(&self.ids);
        shown.push((cells, id));
        id
    }

    /// Whether it is still being drawn, and waited for.
    pub(crate) fn drawing(&self) -> bool {
        matches!(
            *self.state.lock().unwrap_or_else(PoisonError::into_inner),
            State::Drawing(since) if since.elapsed() < PATIENCE
        )
    }

    /// Its image enlarged within `width` columns, centred, on cells of
    /// `cell` pixels: across the width, as large as it reads well, twice
    /// its pixels at most, which its text, drawn larger than mmdr's, bears;
    /// as many rows as that takes. The image is put in `images` to be sent
    /// first. None while it shows no image.
    pub(crate) fn enlarged(
        &self,
        width: usize,
        cell: (u16, u16),
        images: &mut Vec<Image>,
    ) -> Option<Vec<Line<'static>>> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let State::Drawn(drawn) = &*state else {
            return None;
        };
        let pixels = graphics::png_size(&drawn.png)?;
        let room = u16::try_from(width).unwrap_or(u16::MAX);
        let most =
            u16::try_from((2 * pixels.0).div_ceil(u32::from(cell.0.max(1)))).unwrap_or(u16::MAX);
        let (cols, rows) = graphics::across(pixels, cell, room.min(most).max(1), u16::MAX);
        let id = self.id((cols, rows));
        images.push(Image {
            id,
            png: Arc::new(drawn.png.clone()),
            cols,
            rows,
        });
        let indent = [Span::raw(
            " ".repeat(usize::from(room.saturating_sub(cols) / 2)),
        )];
        Some(graphics::cells(id, cols, rows, &indent))
    }

    /// What it shows within `width` columns, after `indent`, on cells of
    /// `cell` pixels: its image's cells, the image put in `images` to be
    /// sent first; else its source. Its image is shown half as large again
    /// as its pixels, one at least [`WIDE`] times wider than tall across
    /// the whole width, and never past [`MOST_ROWS`].
    pub(crate) fn shown(
        &self,
        width: usize,
        indent: &[Span<'static>],
        cell: (u16, u16),
        images: &mut Vec<Image>,
    ) -> Shown {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match &*state {
            State::Drawn(drawn) => {
                let Some(pixels) = graphics::png_size(&drawn.png) else {
                    return Shown::Source(None);
                };
                let room = width.saturating_sub(crate::wrap::spans_width(indent));
                let room = u16::try_from(room).unwrap_or(u16::MAX);
                let larger = if pixels.0 >= WIDE.saturating_mul(pixels.1) {
                    // As large as the width needs, which fit keeps to.
                    let across = u32::from(room) * u32::from(cell.0.max(1));
                    (across * LARGER.1).div_ceil(pixels.0.max(1)).max(LARGER.0)
                } else {
                    LARGER.0
                };
                let shown = (
                    pixels.0.saturating_mul(larger) / LARGER.1,
                    pixels.1.saturating_mul(larger) / LARGER.1,
                );
                let (cols, rows) = graphics::fit(shown, cell, room, MOST_ROWS);
                let id = self.id((cols, rows));
                images.push(Image {
                    id,
                    png: Arc::new(drawn.png.clone()),
                    cols,
                    rows,
                });
                Shown::Image(graphics::cells(id, cols, rows, indent))
            }
            State::Failed(why) => Shown::Source(Some(format!(
                "the diagram could not be drawn: {}",
                cut(why, 300)
            ))),
            State::Drawing(since) if since.elapsed() >= PATIENCE => {
                Shown::Source(Some("the diagram took too long to draw".to_owned()))
            }
            _ => Shown::Source(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Result<Drawn, String>);

    impl Draw for Fixed {
        fn draw(&self, _source: &str) -> Result<Drawn, String> {
            self.0.clone()
        }
    }

    fn drawn(draw: Option<Fixed>) -> Arc<Diagram> {
        let diagram = Diagram::new(
            "graph TD\n A --> B".into(),
            Arc::new(AtomicU32::new(9)),
            draw.map(|d| Arc::new(d) as Arc<dyn Draw>),
        );
        for _ in 0..500 {
            if !diagram.drawing() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        diagram
    }

    #[test]
    fn a_diagram_drawn_is_its_cells_half_as_large_again_and_a_wide_one_takes_the_width() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(200u32.to_be_bytes());
        png.extend(100u32.to_be_bytes());
        let diagram = drawn(Some(Fixed(Ok(Drawn { png }))));
        let mut images = Vec::new();
        let shown = diagram.shown(80, &[Span::raw("  ")], (10, 20), &mut images);
        let Shown::Image(lines) = shown else {
            panic!("{shown:?}")
        };
        // 200 by 100 pixels shown as 300 by 150, on cells of 10 by 20: 30
        // columns, 8 rows; the image alone.
        assert_eq!(lines.len(), 8);
        assert_eq!((images[0].id, images[0].cols, images[0].rows), (9, 30, 8));
        // Three times wider than tall: across the whole width.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(300u32.to_be_bytes());
        png.extend(100u32.to_be_bytes());
        let wide = drawn(Some(Fixed(Ok(Drawn { png }))));
        let mut images = Vec::new();
        wide.shown(80, &[Span::raw("  ")], (10, 20), &mut images);
        assert_eq!((images[0].cols, images[0].rows), (78, 13));
    }

    #[test]
    fn a_diagram_shown_at_another_size_is_another_image() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(200u32.to_be_bytes());
        png.extend(100u32.to_be_bytes());
        let diagram = drawn(Some(Fixed(Ok(Drawn { png }))));
        let mut images = Vec::new();
        for width in [80, 80, 20] {
            diagram.shown(width, &[], (10, 20), &mut images);
        }
        let ids: Vec<(u32, u16)> = images.iter().map(|image| (image.id, image.cols)).collect();
        // The same size, the same image; narrower, another, which the
        // terminal is sent with its own cells.
        assert_eq!(ids, [(9, 30), (9, 30), (10, 20)]);
    }

    #[test]
    fn a_diagram_nothing_draws_is_its_source_and_one_that_failed_says_why() {
        let mut images = Vec::new();
        let mut shown = |diagram: &Diagram| diagram.shown(80, &[], (10, 20), &mut images);
        let source = drawn(None);
        assert!(!source.drawing());
        assert_eq!(shown(&source), Shown::Source(None));
        let failed = drawn(Some(Fixed(Err("mmdr: parse error".into()))));
        assert_eq!(
            shown(&failed),
            Shown::Source(Some(
                "the diagram could not be drawn: mmdr: parse error".into()
            ))
        );
        let not_png = drawn(Some(Fixed(Ok(Drawn {
            png: b"<svg/>".to_vec(),
        }))));
        assert!(
            matches!(shown(&not_png), Shown::Source(Some(why)) if why.ends_with("no PNG image"))
        );
        assert!(images.is_empty());
    }
}
