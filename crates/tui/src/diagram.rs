//! The Mermaid diagrams agents write, drawn as images once whole, in Kitty,
//! by what the program gives the terminal to draw them with ([`Draw`]), as
//! the settings let it; elsewhere, or when nothing draws them, written as
//! their source. A diagram being drawn holds back what its window writes
//! after it, so that the conversation keeps its order.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ratatui::style::Style;
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

/// A diagram drawn, and where its source is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    /// The image, a PNG.
    pub png: Vec<u8>,
    /// The file its source is kept in, for the person to open.
    pub source: Option<PathBuf>,
}

/// The most rows an image takes, so that one fits on screen with what was
/// said of it.
const MOST_ROWS: u16 = 30;

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

/// A diagram of the conversation: the id its image has with the terminal,
/// and where its drawing stands.
#[derive(Debug)]
pub(crate) struct Diagram {
    id: u32,
    state: Mutex<State>,
}

impl Diagram {
    /// The diagram of `source`, which `draw` draws on a thread of its own
    /// when given.
    pub(crate) fn new(source: String, id: u32, draw: Option<Arc<dyn Draw>>) -> Arc<Diagram> {
        let diagram = Arc::new(Diagram {
            id,
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

    /// Whether it is still being drawn, and waited for.
    pub(crate) fn drawing(&self) -> bool {
        matches!(
            *self.state.lock().unwrap_or_else(PoisonError::into_inner),
            State::Drawing(since) if since.elapsed() < PATIENCE
        )
    }

    /// What it shows within `width` columns, after `indent`, on cells of
    /// `cell` pixels: its image's cells, the image put in `images` to be
    /// sent first, and a line in `said` saying where its source is; else
    /// its source.
    pub(crate) fn shown(
        &self,
        width: usize,
        indent: &[Span<'static>],
        cell: (u16, u16),
        said: Style,
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
                let (cols, rows) = graphics::fit(pixels, cell, room, MOST_ROWS);
                images.push(Image {
                    id: self.id,
                    png: Arc::new(drawn.png.clone()),
                    cols,
                    rows,
                });
                let mut lines = graphics::cells(self.id, cols, rows, indent);
                if let Some(path) = &drawn.source {
                    let mut spans = indent.to_vec();
                    spans.push(Span::styled(
                        format!("╰ mermaid, its source in {}", path.display()),
                        said,
                    ));
                    lines.push(Line::from(spans));
                }
                Shown::Image(lines)
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
            9,
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
    fn a_diagram_drawn_is_its_cells_the_image_to_send_and_where_its_source_is() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(200u32.to_be_bytes());
        png.extend(100u32.to_be_bytes());
        let diagram = drawn(Some(Fixed(Ok(Drawn {
            png,
            source: Some("/cache/d.mmd".into()),
        }))));
        let mut images = Vec::new();
        let shown = diagram.shown(80, &[Span::raw("  ")], (10, 20), Style::new(), &mut images);
        let Shown::Image(lines) = shown else {
            panic!("{shown:?}")
        };
        // 200 by 100 pixels on cells of 10 by 20: 20 columns, 5 rows.
        assert_eq!(lines.len(), 6);
        assert_eq!((images[0].id, images[0].cols, images[0].rows), (9, 20, 5));
        assert_eq!(
            lines[5].to_string(),
            "  ╰ mermaid, its source in /cache/d.mmd"
        );
    }

    #[test]
    fn a_diagram_nothing_draws_is_its_source_and_one_that_failed_says_why() {
        let mut images = Vec::new();
        let mut shown =
            |diagram: &Diagram| diagram.shown(80, &[], (10, 20), Style::new(), &mut images);
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
            source: None,
        }))));
        assert!(
            matches!(shown(&not_png), Shown::Source(Some(why)) if why.ends_with("no PNG image"))
        );
        assert!(images.is_empty());
    }
}
