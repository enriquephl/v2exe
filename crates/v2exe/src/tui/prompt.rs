//! A line of input in the bottom border.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
pub struct Prompt {
    /// What the input is for, such as 「过滤」.
    pub label: &'static str,
    pub text: String,
}

/// What a key did to the input.
#[derive(Debug, PartialEq)]
pub enum PromptKey {
    Edited,
    Submit,
    Cancel,
    /// The `Tab` key.
    Complete,
}

impl Prompt {
    /// A printable character is appended, `Backspace` deletes the last character, `Enter`
    /// submits, `Esc` and `←` cancel. Any other key leaves the text as it is and counts as
    /// `Edited`.
    pub fn key(&mut self, key: KeyEvent) -> PromptKey {
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter => return PromptKey::Submit,
            KeyCode::Esc | KeyCode::Left => return PromptKey::Cancel,
            KeyCode::Tab => return PromptKey::Complete,
            KeyCode::Backspace => {
                self.text.pop();
            }
            KeyCode::Char(c) if plain && !c.is_control() => self.text.push(c),
            _ => {}
        }
        PromptKey::Edited
    }

    /// The prompt as a key hint for the bottom border: (label, text).
    pub fn hint(&self) -> (String, String) {
        (format!("{}：", self.label), self.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_editing_and_the_ending_keys() {
        let mut p = Prompt {
            label: "过滤",
            text: String::new(),
        };
        for c in ['p', 'Y', '技', ' '] {
            assert_eq!(p.key(key(KeyCode::Char(c))), PromptKey::Edited);
        }
        assert_eq!(
            p.key(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::SHIFT)),
            PromptKey::Edited
        );
        assert_eq!(p.text, "pY技 $");
        assert_eq!(p.key(key(KeyCode::Backspace)), PromptKey::Edited);
        assert_eq!(p.key(key(KeyCode::Backspace)), PromptKey::Edited);
        assert_eq!(p.text, "pY技");
        assert_eq!(p.key(key(KeyCode::Backspace)), PromptKey::Edited);
        assert_eq!(p.text, "pY");

        assert_eq!(p.key(key(KeyCode::Enter)), PromptKey::Submit);
        assert_eq!(p.key(key(KeyCode::Esc)), PromptKey::Cancel);
        assert_eq!(p.key(key(KeyCode::Left)), PromptKey::Cancel);
        assert_eq!(p.key(key(KeyCode::Tab)), PromptKey::Complete);
        assert_eq!(p.text, "pY");
        assert_eq!(p.hint(), ("过滤：".to_string(), "pY".to_string()));
    }

    #[test]
    fn control_keys_and_an_empty_text_change_nothing() {
        let mut p = Prompt {
            label: "过滤",
            text: String::new(),
        };
        assert_eq!(p.key(key(KeyCode::Backspace)), PromptKey::Edited);
        assert_eq!(
            p.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            PromptKey::Edited
        );
        assert_eq!(p.key(key(KeyCode::Up)), PromptKey::Edited);
        assert_eq!(p.key(key(KeyCode::Char('\u{1b}'))), PromptKey::Edited);
        assert_eq!(p.text, "");
    }
}
