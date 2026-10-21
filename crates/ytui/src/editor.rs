use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
#[derive(Clone, Default)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
}
impl Editor {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }
    pub fn insert(&mut self, value: &str) {
        let cleaned: String = value.chars().filter(|c| !c.is_control()).collect();
        self.text.insert_str(self.cursor, &cleaned);
        self.cursor += cleaned.len();
    }
    pub fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.text[..self.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.text.drain(previous..self.cursor);
                self.cursor = previous;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                let len = self.text[self.cursor..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0);
                self.text.drain(self.cursor..self.cursor + len);
            }
            KeyCode::Left => {
                self.cursor = self.text[..self.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            KeyCode::Right => {
                if let Some(c) = self.text[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8();
                }
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.cursor = 0;
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_edits_preserve_character_boundaries() {
        let mut e = Editor::new("é🐈");
        e.key(KeyCode::Left.into());
        e.key(KeyCode::Backspace.into());
        assert_eq!(e.text, "🐈");
        e.insert("日本");
        assert_eq!(e.text, "日本🐈");
        e.key(KeyCode::Delete.into());
        assert_eq!(e.text, "日本");
    }
}
