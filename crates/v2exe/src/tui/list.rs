//! The cursor shared by the node list and the topic list.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Where the cursor is, which row is at the top of the page, and the digits typed so far.
#[derive(Debug, Default)]
pub struct ListNav {
    pub cursor: usize,
    pub offset: usize,
    /// Digits typed before `Enter`, for jumping to a numbered row.
    pub typed: String,
}

/// What a key did to the list.
#[derive(Debug, PartialEq)]
pub enum Nav {
    /// The key moved the cursor or was taken as a digit.
    Handled,
    /// A number followed by `Enter`; the number as typed, counting from 1.
    Jump(usize),
    Open,
    Back,
    /// Not a list key; the screen decides.
    Pass,
}

impl ListNav {
    /// Handles the moving, paging, first and last row, number and `Enter`, open and back keys
    /// of spec §8.5 「列表通用」. Every other key returns `Pass`. Any key but a digit clears the
    /// typed digits.
    pub fn key(&mut self, key: KeyEvent, len: usize, page: usize) -> Nav {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if let KeyCode::Char(c @ '0'..='9') = key.code
            && plain
        {
            self.typed.push(c);
            return Nav::Handled;
        }
        let typed = std::mem::take(&mut self.typed);
        let last = len.saturating_sub(1);
        let page = page.max(1);
        self.cursor = match key.code {
            KeyCode::Enter if !typed.is_empty() => {
                return typed.parse().map_or(Nav::Handled, Nav::Jump);
            }
            KeyCode::Up => self.cursor.saturating_sub(1),
            KeyCode::Down => (self.cursor + 1).min(last),
            KeyCode::PageUp => self.cursor.saturating_sub(page),
            KeyCode::PageDown => (self.cursor + page).min(last),
            KeyCode::Home => 0,
            KeyCode::End => last,
            KeyCode::Right | KeyCode::Enter => return Nav::Open,
            KeyCode::Left => return Nav::Back,
            KeyCode::Char('b') if ctrl => self.cursor.saturating_sub(page),
            KeyCode::Char('f') if ctrl => (self.cursor + page).min(last),
            KeyCode::Char(c) if plain => match c {
                'k' => self.cursor.saturating_sub(1),
                'j' => (self.cursor + 1).min(last),
                ' ' => (self.cursor + page).min(last),
                '$' => last,
                'l' => return Nav::Open,
                'q' | 'h' => return Nav::Back,
                _ => return Nav::Pass,
            },
            _ => return Nav::Pass,
        };
        self.clamp(len, page);
        Nav::Handled
    }

    /// Keeps `cursor < len` (0 when `len` is 0) and `cursor` within `[offset, offset + page)`.
    /// The page starts no later than needed to fill it to the end of the list.
    pub fn clamp(&mut self, len: usize, page: usize) {
        let page = page.max(1);
        self.cursor = self.cursor.min(len.saturating_sub(1));
        self.offset = self.offset.min(len.saturating_sub(page));
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + page {
            self.offset = self.cursor + 1 - page;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn char(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
    }

    #[test]
    fn movement_paging_and_ends() {
        let mut nav = ListNav::default();
        let (len, page) = (10, 4);
        let mut go = |k: KeyEvent, cursor: usize| {
            assert_eq!(nav.key(k, len, page), Nav::Handled, "{k:?}");
            assert_eq!(nav.cursor, cursor, "{k:?}");
            assert!(
                nav.offset <= nav.cursor && nav.cursor < nav.offset + page,
                "{k:?}: offset {}",
                nav.offset
            );
        };
        go(char('j'), 1);
        go(key(KeyCode::Down), 2);
        go(char('k'), 1);
        go(key(KeyCode::Up), 0);
        go(key(KeyCode::Up), 0);
        go(key(KeyCode::End), 9);
        go(key(KeyCode::Down), 9);
        go(key(KeyCode::Home), 0);
        go(char('$'), 9);
        go(key(KeyCode::Home), 0);

        go(key(KeyCode::PageDown), 4);
        go(char(' '), 8);
        go(ctrl('f'), 9);
        go(key(KeyCode::PageUp), 5);
        go(ctrl('b'), 1);
        go(key(KeyCode::PageUp), 0);
    }

    #[test]
    fn digits_then_enter_jump_and_other_keys_clear_them() {
        let mut nav = ListNav::default();
        assert_eq!(nav.key(char('1'), 20, 5), Nav::Handled);
        assert_eq!(nav.key(char('2'), 20, 5), Nav::Handled);
        assert_eq!(nav.typed, "12");
        assert_eq!(nav.key(key(KeyCode::Enter), 20, 5), Nav::Jump(12));
        assert_eq!(nav.typed, "");
        assert_eq!(nav.cursor, 0);

        assert_eq!(nav.key(char('3'), 20, 5), Nav::Handled);
        assert_eq!(nav.key(char('j'), 20, 5), Nav::Handled);
        assert_eq!(nav.typed, "");
        assert_eq!(nav.cursor, 1);

        nav.key(char('3'), 20, 5);
        assert_eq!(nav.key(key(KeyCode::Esc), 20, 5), Nav::Pass);
        assert_eq!(nav.typed, "");
        nav.key(char('3'), 20, 5);
        assert_eq!(nav.key(char('y'), 20, 5), Nav::Pass);
        assert_eq!(nav.typed, "");

        assert_eq!(nav.key(key(KeyCode::Enter), 20, 5), Nav::Open);
        assert_eq!(nav.key(key(KeyCode::Right), 20, 5), Nav::Open);
        assert_eq!(nav.key(char('l'), 20, 5), Nav::Open);
        assert_eq!(nav.key(key(KeyCode::Left), 20, 5), Nav::Back);
        assert_eq!(nav.key(char('q'), 20, 5), Nav::Back);
        assert_eq!(nav.key(char('h'), 20, 5), Nav::Back);

        // A number too large to be a row number jumps nowhere.
        for _ in 0..30 {
            nav.key(char('9'), 20, 5);
        }
        assert_eq!(nav.key(key(KeyCode::Enter), 20, 5), Nav::Handled);
        assert_eq!(nav.typed, "");
    }

    #[test]
    fn letters_with_control_are_not_list_keys() {
        let mut nav = ListNav::default();
        assert_eq!(nav.key(ctrl('j'), 10, 4), Nav::Pass);
        assert_eq!(nav.key(ctrl('l'), 10, 4), Nav::Pass);
        assert_eq!(nav.key(ctrl('q'), 10, 4), Nav::Pass);
        assert_eq!(nav.cursor, 0);
    }

    #[test]
    fn an_empty_list_ignores_movement() {
        let mut nav = ListNav::default();
        for k in [
            char('j'),
            char('k'),
            key(KeyCode::Down),
            key(KeyCode::Up),
            key(KeyCode::PageDown),
            key(KeyCode::PageUp),
            char(' '),
            ctrl('f'),
            ctrl('b'),
            key(KeyCode::Home),
            key(KeyCode::End),
            char('$'),
        ] {
            nav.key(k, 0, 4);
            assert_eq!((nav.cursor, nav.offset), (0, 0), "{k:?}");
            nav.key(k, 0, 0);
            assert_eq!((nav.cursor, nav.offset), (0, 0), "{k:?}");
        }
        nav.clamp(0, 0);
        assert_eq!((nav.cursor, nav.offset), (0, 0));
    }

    #[test]
    fn clamp_pulls_the_cursor_back_after_the_list_or_the_page_shrinks() {
        let mut nav = ListNav {
            cursor: 9,
            offset: 6,
            typed: String::new(),
        };
        nav.clamp(5, 2);
        assert_eq!((nav.cursor, nav.offset), (4, 3));

        // A smaller page moves the page to the cursor.
        let mut nav = ListNav {
            cursor: 9,
            offset: 0,
            typed: String::new(),
        };
        nav.clamp(20, 4);
        assert_eq!((nav.cursor, nav.offset), (9, 6));

        // A larger page fills up to the end of the list.
        let mut nav = ListNav {
            cursor: 99,
            offset: 92,
            typed: String::new(),
        };
        nav.clamp(100, 22);
        assert_eq!((nav.cursor, nav.offset), (99, 78));

        // A page of 0 rows is taken as 1.
        nav.clamp(100, 0);
        assert_eq!((nav.cursor, nav.offset), (99, 99));
    }
}
