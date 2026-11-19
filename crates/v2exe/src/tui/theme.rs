//! Semantic roles mapped to terminal styles.
//!
//! Only the terminal's 16 named colors are used, so the interface follows the terminal's theme.
//! With color off, roles are told apart by reversed, bold and dim alone.

use ratatui::style::{Color, Modifier, Style};

/// What a piece of text means; [`Theme::style`] decides how it looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    /// Body text.
    Plain,
    Bold,
    /// Minor information: dates, numbers, floor numbers.
    Minor,
    /// Where things are: node names, keys.
    Location,
    /// New: the unread mark.
    Fresh,
    /// Changed: new replies since the topic was read.
    Changed,
    /// Errors.
    Alert,
    /// People's names.
    Person,
    /// The original-poster mark.
    Op,
    Link,
    Mention,
    Code,
    /// The bar at the left of a code block.
    CodeBar,
    Quote,
    Image,
    /// The cursor glyph.
    Cursor,
    /// The row the cursor is on.
    Selected,
    /// Reply counts of 1 to 9.
    RepliesFew,
    /// Reply counts of 10 to 99.
    RepliesMany,
    /// Reply counts of 100 or more.
    RepliesHuge,
}

/// Maps [`Paint`] to a style.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// False under `NO_COLOR`: styles then carry no color at all.
    pub color: bool,
}

impl Theme {
    pub fn style(&self, paint: Paint) -> Style {
        if self.color {
            colored(paint)
        } else {
            mono(paint)
        }
    }
}

fn colored(paint: Paint) -> Style {
    let fg = |color| Style::new().fg(color);
    let bold = |color| Style::new().fg(color).add_modifier(Modifier::BOLD);
    match paint {
        Paint::Plain | Paint::Code => Style::new(),
        Paint::Bold | Paint::Selected => Style::new().add_modifier(Modifier::BOLD),
        Paint::Minor | Paint::Quote => Style::new().add_modifier(Modifier::DIM),
        Paint::Location | Paint::Link | Paint::Mention | Paint::Image => fg(Color::Blue),
        Paint::Fresh | Paint::CodeBar | Paint::RepliesFew => fg(Color::Green),
        Paint::Cursor => bold(Color::Green),
        Paint::Changed => fg(Color::Yellow),
        Paint::RepliesMany => bold(Color::Yellow),
        Paint::Alert => fg(Color::Red),
        Paint::RepliesHuge => bold(Color::Red),
        Paint::Person => fg(Color::Cyan),
        Paint::Op => fg(Color::Magenta),
    }
}

fn mono(paint: Paint) -> Style {
    match paint {
        Paint::Plain | Paint::Person | Paint::Link | Paint::Code | Paint::RepliesFew => {
            Style::new()
        }
        Paint::Minor | Paint::Quote | Paint::Image | Paint::CodeBar => {
            Style::new().add_modifier(Modifier::DIM)
        }
        Paint::RepliesHuge => Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        Paint::Bold
        | Paint::Location
        | Paint::Fresh
        | Paint::Changed
        | Paint::Alert
        | Paint::Op
        | Paint::Mention
        | Paint::Cursor
        | Paint::Selected
        | Paint::RepliesMany => Style::new().add_modifier(Modifier::BOLD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Paint; 20] = [
        Paint::Plain,
        Paint::Bold,
        Paint::Minor,
        Paint::Location,
        Paint::Fresh,
        Paint::Changed,
        Paint::Alert,
        Paint::Person,
        Paint::Op,
        Paint::Link,
        Paint::Mention,
        Paint::Code,
        Paint::CodeBar,
        Paint::Quote,
        Paint::Image,
        Paint::Cursor,
        Paint::Selected,
        Paint::RepliesFew,
        Paint::RepliesMany,
        Paint::RepliesHuge,
    ];

    #[test]
    fn without_color_styles_only_use_reversed_bold_and_dim() {
        let theme = Theme { color: false };
        for paint in ALL {
            let style = theme.style(paint);
            assert_eq!(
                (style.fg, style.bg, style.underline_color),
                (None, None, None)
            );
            assert!(
                (Modifier::REVERSED | Modifier::BOLD | Modifier::DIM).contains(style.add_modifier),
                "{paint:?}"
            );
            assert!(style.sub_modifier.is_empty(), "{paint:?}");
        }
    }
}
