//! Pictures drawn by the terminal itself, in Kitty: the diagrams agents
//! write, once drawn as images. Kitty's graphics protocol with Unicode
//! placeholders: an image is sent once, then shown under cells of a
//! placeholder character written as any text, so that it scrolls into the
//! terminal's own scrollback with the conversation and stays there. Inside
//! tmux, what is sent is passed through.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// The terminal iq draws in, when it is Kitty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Kitty {
    /// Inside tmux, which passes on what it is asked to (with
    /// `allow-passthrough on`).
    tmux: bool,
}

/// Kitty, when its variables (`var`) say the terminal is: it sets
/// `KITTY_WINDOW_ID` in every shell it starts, and `TERM` unless the person
/// changed it.
pub(crate) fn detect(var: impl Fn(&str) -> Option<String>) -> Option<Kitty> {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    (set("KITTY_WINDOW_ID") || var("TERM").as_deref() == Some("xterm-kitty"))
        .then(|| Kitty { tmux: set("TMUX") })
}

/// The character Kitty shows an image under.
const PLACEHOLDER: char = '\u{10EEEE}';

/// The marks after a placeholder that say its row and column in the image:
/// the first marks the first, and so on. Kitty's own list,
/// `rowcolumn-diacritics.txt`.
const DIACRITICS: [char; 297] = [
    '\u{0305}',
    '\u{030D}',
    '\u{030E}',
    '\u{0310}',
    '\u{0312}',
    '\u{033D}',
    '\u{033E}',
    '\u{033F}',
    '\u{0346}',
    '\u{034A}',
    '\u{034B}',
    '\u{034C}',
    '\u{0350}',
    '\u{0351}',
    '\u{0352}',
    '\u{0357}',
    '\u{035B}',
    '\u{0363}',
    '\u{0364}',
    '\u{0365}',
    '\u{0366}',
    '\u{0367}',
    '\u{0368}',
    '\u{0369}',
    '\u{036A}',
    '\u{036B}',
    '\u{036C}',
    '\u{036D}',
    '\u{036E}',
    '\u{036F}',
    '\u{0483}',
    '\u{0484}',
    '\u{0485}',
    '\u{0486}',
    '\u{0487}',
    '\u{0592}',
    '\u{0593}',
    '\u{0594}',
    '\u{0595}',
    '\u{0597}',
    '\u{0598}',
    '\u{0599}',
    '\u{059C}',
    '\u{059D}',
    '\u{059E}',
    '\u{059F}',
    '\u{05A0}',
    '\u{05A1}',
    '\u{05A8}',
    '\u{05A9}',
    '\u{05AB}',
    '\u{05AC}',
    '\u{05AF}',
    '\u{05C4}',
    '\u{0610}',
    '\u{0611}',
    '\u{0612}',
    '\u{0613}',
    '\u{0614}',
    '\u{0615}',
    '\u{0616}',
    '\u{0617}',
    '\u{0657}',
    '\u{0658}',
    '\u{0659}',
    '\u{065A}',
    '\u{065B}',
    '\u{065D}',
    '\u{065E}',
    '\u{06D6}',
    '\u{06D7}',
    '\u{06D8}',
    '\u{06D9}',
    '\u{06DA}',
    '\u{06DB}',
    '\u{06DC}',
    '\u{06DF}',
    '\u{06E0}',
    '\u{06E1}',
    '\u{06E2}',
    '\u{06E4}',
    '\u{06E7}',
    '\u{06E8}',
    '\u{06EB}',
    '\u{06EC}',
    '\u{0730}',
    '\u{0732}',
    '\u{0733}',
    '\u{0735}',
    '\u{0736}',
    '\u{073A}',
    '\u{073D}',
    '\u{073F}',
    '\u{0740}',
    '\u{0741}',
    '\u{0743}',
    '\u{0745}',
    '\u{0747}',
    '\u{0749}',
    '\u{074A}',
    '\u{07EB}',
    '\u{07EC}',
    '\u{07ED}',
    '\u{07EE}',
    '\u{07EF}',
    '\u{07F0}',
    '\u{07F1}',
    '\u{07F3}',
    '\u{0816}',
    '\u{0817}',
    '\u{0818}',
    '\u{0819}',
    '\u{081B}',
    '\u{081C}',
    '\u{081D}',
    '\u{081E}',
    '\u{081F}',
    '\u{0820}',
    '\u{0821}',
    '\u{0822}',
    '\u{0823}',
    '\u{0825}',
    '\u{0826}',
    '\u{0827}',
    '\u{0829}',
    '\u{082A}',
    '\u{082B}',
    '\u{082C}',
    '\u{082D}',
    '\u{0951}',
    '\u{0953}',
    '\u{0954}',
    '\u{0F82}',
    '\u{0F83}',
    '\u{0F86}',
    '\u{0F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

/// The most cells an image takes either way: as many as there are marks.
const MAX_CELLS: u16 = 297;

impl Kitty {
    /// What makes the terminal keep `png` as image `id`, and show it on
    /// `cols` by `rows` cells wherever its placeholders are, keeping its
    /// shape.
    pub(crate) fn send(&self, id: u32, png: &[u8], cols: u16, rows: u16) -> String {
        let data = STANDARD.encode(png);
        // Kitty takes at most 4096 bytes at a time.
        let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
        let mut out = String::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let more = u8::from(i + 1 < chunks.len());
            let keys = if i == 0 {
                format!("a=t,f=100,t=d,i={id},q=2,m={more}")
            } else {
                format!("q=2,m={more}")
            };
            let chunk = String::from_utf8_lossy(chunk);
            out.push_str(&self.wrap(&format!("\x1b_G{keys};{chunk}\x1b\\")));
        }
        out.push_str(&self.wrap(&format!(
            "\x1b_Ga=p,U=1,i={id},p=1,c={cols},r={rows},q=2\x1b\\"
        )));
        out
    }

    /// `escape` as tmux passes it on: inside its own, each escape character
    /// doubled.
    fn wrap(&self, escape: &str) -> String {
        if self.tmux {
            format!("\x1bPtmux;{}\x1b\\", escape.replace('\x1b', "\x1b\x1b"))
        } else {
            escape.to_owned()
        }
    }
}

/// The lines of cells image `id` is shown under, `cols` by `rows`, after
/// `indent`: each cell a placeholder marked with its row and column, its
/// colour the id.
pub(crate) fn cells(id: u32, cols: u16, rows: u16, indent: &[Span<'static>]) -> Vec<Line<'static>> {
    let [_, r, g, b] = id.to_be_bytes();
    let style = Style::new().fg(Color::Rgb(r, g, b));
    let (cols, rows) = (cols.min(MAX_CELLS), rows.min(MAX_CELLS));
    (0..usize::from(rows))
        .map(|row| {
            let mut text = String::new();
            for column in &DIACRITICS[..usize::from(cols)] {
                text.push(PLACEHOLDER);
                text.push(DIACRITICS[row]);
                text.push(*column);
            }
            let mut spans = indent.to_vec();
            spans.push(Span::styled(text, style));
            Line::from(spans)
        })
        .collect()
}

/// The width and height of a PNG image, from its first bytes; none when
/// they are not a PNG's.
pub(crate) fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if head.len() < 24 || !head.starts_with(SIGNATURE) || &head[12..16] != b"IHDR" {
        return None;
    }
    let number =
        |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let (width, height) = (number(16), number(20));
    (width > 0 && height > 0).then_some((width, height))
}

/// The cells an image of `pixels` takes on cells of `cell` pixels: as many
/// as its own size needs, at most `width` columns and `height` rows,
/// keeping its shape.
pub(crate) fn fit(pixels: (u32, u32), cell: (u16, u16), width: u16, height: u16) -> (u16, u16) {
    let (w, h) = (u64::from(pixels.0.max(1)), u64::from(pixels.1.max(1)));
    let (cw, ch) = (u64::from(cell.0.max(1)), u64::from(cell.1.max(1)));
    let width = u64::from(width.clamp(1, MAX_CELLS));
    let height = u64::from(height.clamp(1, MAX_CELLS));
    let mut cols = w.div_ceil(cw).clamp(1, width);
    let mut rows = (cols * cw * h).div_ceil(w * ch).max(1);
    if rows > height {
        rows = height;
        cols = (rows * ch * w).div_ceil(h * cw).clamp(1, cols);
    }
    // Both are at most MAX_CELLS by now.
    (
        u16::try_from(cols).unwrap_or(MAX_CELLS),
        u16::try_from(rows).unwrap_or(MAX_CELLS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn kitty_is_known_by_its_variables() {
        let kitty = Some(Kitty { tmux: false });
        assert_eq!(detect(env(&[("KITTY_WINDOW_ID", "3")])), kitty);
        assert_eq!(detect(env(&[("TERM", "xterm-kitty")])), kitty);
        assert_eq!(detect(env(&[("TERM", "xterm-ghostty")])), None);
        assert_eq!(detect(env(&[("TERM", "xterm-256color")])), None);
        let tmux = detect(env(&[("KITTY_WINDOW_ID", "3"), ("TMUX", "/t,1,0")]));
        assert_eq!(tmux, Some(Kitty { tmux: true }));
    }

    #[test]
    fn an_image_is_sent_in_chunks_then_placed_and_tmux_passes_it_on() {
        let out = Kitty { tmux: false }.send(7, &[0; 4000], 10, 4);
        let escapes: Vec<&str> = out.split("\x1b\\").filter(|s| !s.is_empty()).collect();
        assert_eq!(escapes.len(), 3);
        assert!(escapes[0].starts_with("\x1b_Ga=t,f=100,t=d,i=7,q=2,m=1;"));
        assert_eq!(escapes[0].split(';').nth(1).unwrap().len(), 4096);
        assert!(escapes[1].starts_with("\x1b_Gq=2,m=0;"));
        assert_eq!(escapes[2], "\x1b_Ga=p,U=1,i=7,p=1,c=10,r=4,q=2");
        let wrapped = Kitty { tmux: true }.send(2, &[1], 1, 1);
        assert!(wrapped.starts_with("\x1bPtmux;\x1b\x1b_Ga=t"));
    }

    #[test]
    fn cells_are_one_column_wide_and_carry_their_row_column_and_image() {
        let lines = cells(0x01_02_03, 3, 2, &[Span::raw("  ")]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].width(), 5);
        let text = lines[1].spans[1].content.as_ref();
        assert!(text.starts_with("\u{10EEEE}\u{030D}\u{0305}"));
        assert_eq!(lines[1].spans[1].style.fg, Some(Color::Rgb(1, 2, 3)));
    }

    #[test]
    fn a_png_says_its_size_and_keeps_its_shape_within_the_room() {
        let mut head = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        head.extend(838u32.to_be_bytes());
        head.extend(140u32.to_be_bytes());
        assert_eq!(png_size(&head), Some((838, 140)));
        assert_eq!(png_size(b"GIF89a, not a png at all"), None);
        assert_eq!(fit((800, 160), (10, 20), 200, 40), (80, 8));
        assert_eq!(fit((800, 160), (10, 20), 40, 40), (40, 4));
        assert_eq!(fit((100, 1000), (10, 20), 80, 10), (2, 10));
        assert_eq!(fit((5, 5), (10, 20), 80, 10), (1, 1));
    }
}
