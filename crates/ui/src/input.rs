//! What the person writes: a text and where the cursor stands in it, edited
//! with the keys a terminal's line editors take (Emacs's, which readline
//! and every shell share), and the messages sent before, to go back to.

use crate::keys::Chord;

/// The most messages kept to go back to.
const HISTORY: usize = 200;

/// A text being written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Editor {
    text: String,
    /// Where the cursor stands, in bytes, always between two characters.
    cursor: usize,
    /// The messages sent, oldest first.
    history: Vec<String>,
    /// Which of them shows while the person goes back, if one does.
    back: Option<usize>,
    /// What was written before going back.
    draft: String,
}

impl Editor {
    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the cursor stands, in bytes.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether nothing is written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Writes `text` where the cursor stands, as typed or pasted: a
    /// return a pasted text carries becomes a new line.
    pub fn insert(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.back = None;
    }

    /// Replaces the text, the cursor at its end.
    pub fn set(&mut self, text: &str) {
        self.text = text.to_owned();
        self.cursor = self.text.len();
        self.back = None;
    }

    /// Clears the text.
    pub fn clear(&mut self) {
        self.set("");
    }

    /// Takes the text to send it, kept to go back to.
    pub fn take(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.back = None;
        self.draft.clear();
        if !text.trim().is_empty() && self.history.last() != Some(&text) {
            self.history.push(text.clone());
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        text
    }

    /// Edits as `chord` says, when it is an editing key. Returns whether
    /// it was one.
    pub fn key(&mut self, chord: &Chord) -> bool {
        let word = chord.alt || chord.ctrl;
        match (chord.ctrl, chord.alt, chord.key.as_str()) {
            (_, _, "Backspace") if word => self.delete_word_back(),
            (_, _, "Backspace") | (true, false, "H") => self.delete_back(),
            (true, false, "W") => self.delete_word_back(),
            (_, _, "Delete") | (true, false, "D") => self.delete_forward(),
            (_, _, "Left") if word => self.cursor = self.word_left(),
            (_, _, "Right") if word => self.cursor = self.word_right(),
            (false, true, "B") => self.cursor = self.word_left(),
            (false, true, "F") => self.cursor = self.word_right(),
            (false, false, "Left") | (true, false, "B") => self.cursor = self.left(),
            (false, false, "Right") | (true, false, "F") => self.cursor = self.right(),
            (_, _, "Home") | (true, false, "A") => self.cursor = self.line_start(),
            (_, _, "End") | (true, false, "E") => self.cursor = self.line_end(),
            (true, false, "K") => {
                let end = self.line_end();
                let end = if end == self.cursor && end < self.text.len() {
                    end + 1
                } else {
                    end
                };
                self.text.replace_range(self.cursor..end, "");
            }
            (true, false, "U") => {
                let start = self.line_start();
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            (false, false, "Up") => self.up(),
            (false, false, "Down") => self.down(),
            _ => return false,
        }
        true
    }

    fn left(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at)
    }

    fn right(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    /// The start of the word before the cursor: past what separates words,
    /// then past the word.
    fn word_left(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end_matches(|c: char| !c.is_alphanumeric());
        trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| !c.is_alphanumeric())
            .map_or(0, |(at, c)| at + c.len_utf8())
    }

    /// The end of the word after the cursor.
    fn word_right(&self) -> usize {
        let after = &self.text[self.cursor..];
        let skipped = after.len()
            - after
                .trim_start_matches(|c: char| !c.is_alphanumeric())
                .len();
        let rest = &after[skipped..];
        let word = rest
            .char_indices()
            .find(|(_, c)| !c.is_alphanumeric())
            .map_or(rest.len(), |(at, _)| at);
        self.cursor + skipped + word
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |at| at + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |at| self.cursor + at)
    }

    fn delete_back(&mut self) {
        let from = self.left();
        self.text.replace_range(from..self.cursor, "");
        self.cursor = from;
    }

    fn delete_forward(&mut self) {
        let to = self.right();
        self.text.replace_range(self.cursor..to, "");
    }

    fn delete_word_back(&mut self) {
        let from = self.word_left();
        self.text.replace_range(from..self.cursor, "");
        self.cursor = from;
    }

    /// Where the cursor is: its line, and its column in characters.
    #[must_use]
    pub fn position(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let line = before.matches('\n').count();
        let column = before[self.line_start()..].chars().count();
        (line, column)
    }

    /// The cursor on the line above at the same column, or the message
    /// sent before when it is on the first line.
    fn up(&mut self) {
        let (line, column) = self.position();
        if line == 0 {
            self.older();
            return;
        }
        let start = self.line_start();
        let above_start = self.text[..start - 1].rfind('\n').map_or(0, |at| at + 1);
        self.cursor = self.at_column(above_start, column);
    }

    /// The cursor on the line below, or the message sent after when it is
    /// on the last line.
    fn down(&mut self) {
        let end = self.line_end();
        if end == self.text.len() {
            self.newer();
            return;
        }
        let (_, column) = self.position();
        self.cursor = self.at_column(end + 1, column);
    }

    /// The place `column` characters into the line starting at `start`, or
    /// its end.
    fn at_column(&self, start: usize, column: usize) -> usize {
        let line = &self.text[start..];
        let line = &line[..line.find('\n').unwrap_or(line.len())];
        start
            + line
                .char_indices()
                .nth(column)
                .map_or(line.len(), |(at, _)| at)
    }

    fn older(&mut self) {
        let at = match self.back {
            Some(0) => return,
            Some(at) => at - 1,
            None if self.history.is_empty() => return,
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
        };
        self.back = Some(at);
        self.text = self.history[at].clone();
        self.cursor = self.text.len();
    }

    fn newer(&mut self) {
        let Some(at) = self.back else {
            return;
        };
        if at + 1 < self.history.len() {
            self.back = Some(at + 1);
            self.text = self.history[at + 1].clone();
        } else {
            self.back = None;
            self.text = std::mem::take(&mut self.draft);
        }
        self.cursor = self.text.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).unwrap()
    }

    #[test]
    fn keys_move_by_character_word_and_line() {
        let mut editor = Editor::default();
        editor.insert("fix the café\r\nthen test");
        assert_eq!(editor.text(), "fix the café\nthen test");
        assert_eq!(editor.position(), (1, 9));
        assert!(editor.key(&chord("ctrl+a")));
        assert_eq!(editor.position(), (1, 0));
        editor.key(&chord("up"));
        assert_eq!(editor.position(), (0, 0));
        editor.key(&chord("alt+f"));
        editor.key(&chord("alt+f"));
        assert_eq!(editor.position(), (0, 7));
        editor.key(&chord("ctrl+e"));
        editor.key(&chord("backspace"));
        assert_eq!(editor.text(), "fix the caf\nthen test");
        editor.key(&chord("ctrl+w"));
        assert_eq!(editor.text(), "fix the \nthen test");
        editor.key(&chord("ctrl+k"));
        assert_eq!(editor.text(), "fix the then test");
        editor.key(&chord("ctrl+u"));
        assert_eq!(editor.text(), "then test");
        assert!(!editor.key(&chord("ctrl+o")));
    }

    #[test]
    fn sent_messages_come_back_and_the_draft_with_them() {
        let mut editor = Editor::default();
        for sent in ["first", "second", "second"] {
            editor.insert(sent);
            assert_eq!(editor.take(), sent);
        }
        editor.insert("draft");
        editor.key(&chord("up"));
        assert_eq!(editor.text(), "second");
        editor.key(&chord("up"));
        assert_eq!(editor.text(), "first");
        editor.key(&chord("up"));
        assert_eq!(editor.text(), "first");
        editor.key(&chord("down"));
        editor.key(&chord("down"));
        assert_eq!(editor.text(), "draft");
    }
}
