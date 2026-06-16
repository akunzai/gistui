use crossterm::event::KeyCode;
use std::ops::Deref;

/// A single-line text buffer with a char-indexed cursor, shared by every inline text
/// input (the gist description editor and the `/` filters). The cursor is kept within
/// `[0, char_len]` and always on a char boundary, so multi-byte (e.g. CJK) input is safe.
///
/// `Deref<Target = str>` and `Display` let read sites treat it like the old `String`
/// (`is_empty`, `to_lowercase`, `&input` as `&str`, `format!("{input}")`); only the
/// mutating paths go through the explicit editing methods.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    /// Number of chars before the cursor (0 = line start, char_len = line end).
    cursor: usize,
}

/// What [`TextInput::apply_edit`] did with a key, so filter callers can tell a text
/// change (which must re-rank/reset) from a pure cursor move (which must not) from a
/// key the input didn't consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditResult {
    /// The text changed (insert or delete).
    Changed,
    /// Only the cursor moved.
    Moved,
    /// Not an editing key; the caller should handle it.
    Ignored,
}

impl TextInput {
    /// Cursor position as a char count from the line start.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn char_len(&self) -> usize {
        self.value.chars().count()
    }

    /// Byte offset of char index `i` (clamped to the end for `i >= char_len`).
    fn byte_at(&self, i: usize) -> usize {
        self.value
            .char_indices()
            .nth(i)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    /// Insert `c` at the cursor and advance past it.
    pub fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    /// Delete the char before the cursor; returns whether anything was removed.
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let start = self.byte_at(self.cursor - 1);
        let end = self.byte_at(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
        true
    }

    /// Delete the char at the cursor; returns whether anything was removed.
    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.char_len() {
            return false;
        }
        let start = self.byte_at(self.cursor);
        let end = self.byte_at(self.cursor + 1);
        self.value.replace_range(start..end, "");
        true
    }

    /// Move the cursor one char left; returns whether it moved.
    pub fn left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// Move the cursor one char right; returns whether it moved.
    pub fn right(&mut self) -> bool {
        if self.cursor >= self.char_len() {
            return false;
        }
        self.cursor += 1;
        true
    }

    /// Move the cursor to the line start; returns whether it moved.
    pub fn home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    /// Move the cursor to the line end; returns whether it moved.
    pub fn end(&mut self) -> bool {
        let len = self.char_len();
        if self.cursor == len {
            return false;
        }
        self.cursor = len;
        true
    }

    /// Clear the text and reset the cursor to the start.
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Replace the whole value, placing the cursor at the end (used when opening the
    /// description editor pre-filled with the gist's current description).
    pub fn set(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.char_len();
    }

    /// Apply one editing key. `Esc`/`Enter` are intentionally NOT handled here — each
    /// caller owns those (filters clear/exit; the description editor applies/cancels).
    pub fn apply_edit(&mut self, code: KeyCode) -> EditResult {
        match code {
            KeyCode::Char(c) => {
                self.insert(c);
                EditResult::Changed
            }
            KeyCode::Backspace => bool_to_change(self.backspace()),
            KeyCode::Delete => bool_to_change(self.delete()),
            KeyCode::Left => bool_to_move(self.left()),
            KeyCode::Right => bool_to_move(self.right()),
            KeyCode::Home => bool_to_move(self.home()),
            KeyCode::End => bool_to_move(self.end()),
            _ => EditResult::Ignored,
        }
    }
}

fn bool_to_change(changed: bool) -> EditResult {
    if changed {
        EditResult::Changed
    } else {
        EditResult::Ignored
    }
}

fn bool_to_move(moved: bool) -> EditResult {
    if moved {
        EditResult::Moved
    } else {
        EditResult::Ignored
    }
}

impl Deref for TextInput {
    type Target = str;
    fn deref(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Display for TextInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value)
    }
}

impl From<&str> for TextInput {
    fn from(value: &str) -> Self {
        let mut input = Self::default();
        input.set(value.to_string());
        input
    }
}

impl From<String> for TextInput {
    fn from(value: String) -> Self {
        let mut input = Self::default();
        input.set(value);
        input
    }
}

impl PartialEq<str> for TextInput {
    fn eq(&self, other: &str) -> bool {
        self.value == other
    }
}

impl PartialEq<&str> for TextInput {
    fn eq(&self, other: &&str) -> bool {
        self.value == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> TextInput {
        let mut input = TextInput::default();
        for c in s.chars() {
            input.insert(c);
        }
        input
    }

    #[test]
    fn insert_at_cursor_after_moving_left() {
        let mut input = typed("ac");
        assert!(input.left()); // between a|c
        input.insert('b');
        assert_eq!(&*input, "abc");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn backspace_removes_char_before_cursor() {
        let mut input = typed("abc");
        assert!(input.left()); // ab|c
        assert!(input.backspace()); // a|c
        assert_eq!(&*input, "ac");
        assert_eq!(input.cursor(), 1);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut input = typed("ab");
        input.home();
        assert!(!input.backspace());
        assert_eq!(&*input, "ab");
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut input = typed("abc");
        input.home(); // |abc
        assert!(input.delete()); // |bc
        assert_eq!(&*input, "bc");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut input = typed("ab");
        assert!(!input.delete());
        assert_eq!(&*input, "ab");
    }

    #[test]
    fn cursor_movement_clamps_at_bounds() {
        let mut input = typed("ab");
        assert!(!input.right()); // already at end
        assert!(input.home());
        assert!(!input.left()); // already at start
        assert!(input.end());
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn edits_respect_multibyte_char_boundaries() {
        // Two CJK chars (3 bytes each); editing must not slice a UTF-8 sequence.
        let mut input = typed("中文");
        assert_eq!(input.cursor(), 2);
        assert!(input.left()); // 中|文
        input.insert('x'); // 中x文
        assert_eq!(&*input, "中x文");
        assert!(input.backspace()); // 中|文
        assert_eq!(&*input, "中文");
        assert_eq!(input.cursor(), 1);
    }

    #[test]
    fn set_places_cursor_at_end_and_clear_resets() {
        let mut input = TextInput::default();
        input.set("hello");
        assert_eq!(input.cursor(), 5);
        input.clear();
        assert_eq!(&*input, "");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn apply_edit_classifies_keys() {
        let mut input = typed("ab");
        assert_eq!(input.apply_edit(KeyCode::Char('c')), EditResult::Changed);
        assert_eq!(input.apply_edit(KeyCode::Left), EditResult::Moved);
        assert_eq!(input.apply_edit(KeyCode::Backspace), EditResult::Changed);
        assert_eq!(input.apply_edit(KeyCode::Esc), EditResult::Ignored);
        // Backspace with nothing before the cursor is a no-op, not a change.
        input.home();
        assert_eq!(input.apply_edit(KeyCode::Backspace), EditResult::Ignored);
    }
}
