//! The rounded panel every screen is drawn in, with colored segments in the top border and key
//! hints and the position in the bottom border.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::theme::{Paint, Theme};
use crate::render::sanitize;
use crate::store::config::SegmentEdge;

/// What a segment is about; decides its background color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Site,
    Location,
    Status,
    Ok,
    Warn,
    Alert,
    Identity,
}

/// A block of one background color in the top border.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub text: String,
    pub tone: Tone,
}

/// What the border around a screen says.
#[derive(Debug, Clone, PartialEq)]
pub struct Chrome {
    /// Top border, left group.
    pub left: Vec<Segment>,
    /// Top border, right group.
    pub right: Vec<Segment>,
    /// Bottom border, left: (key, description).
    pub hints: Vec<(String, String)>,
    /// Bottom border, right, such as "3/50" or "62%".
    pub position: String,
}

/// The smallest terminal the interface is drawn in: (columns, rows).
pub const MIN_SIZE: (u16, u16) = (40, 8);

/// Draws the border and returns the area inside it. Right-group segments are dropped from the
/// last one until at least one border cell is left between the two groups, and hints from the
/// last one until they fit beside the position; the left group and the position stay, cut at
/// the corners if they alone are too wide.
pub fn draw_frame(
    frame: &mut Frame,
    area: Rect,
    chrome: &Chrome,
    theme: &Theme,
    edge: SegmentEdge,
) -> Rect {
    let block = Block::bordered().border_type(BorderType::Rounded);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if area.width > 2 && area.height >= 2 {
        draw_top(frame.buffer_mut(), area, chrome, theme, edge);
        draw_bottom(frame.buffer_mut(), area, chrome, theme);
    }
    inner
}

/// The notice drawn when the terminal is smaller than [`MIN_SIZE`], on the first row and cut at
/// the area's width.
pub fn draw_too_small(frame: &mut Frame, area: Rect) {
    let notice = format!("终端太小，至少需要 {}×{}", MIN_SIZE.0, MIN_SIZE.1);
    frame.render_widget(Paragraph::new(notice), area);
}

/// Whether the interface can be drawn in `area`.
pub fn fits(area: Rect) -> bool {
    area.width >= MIN_SIZE.0 && area.height >= MIN_SIZE.1
}

/// Arrow after a left-group segment.
const LEFT_ARROW: &str = "\u{e0b0}";
/// Arrow before a right-group segment.
const RIGHT_ARROW: &str = "\u{e0b2}";
/// Gap between two key hints.
const HINT_GAP: &str = "   ";

fn tone_color(tone: Tone) -> Color {
    match tone {
        Tone::Site => Color::Cyan,
        Tone::Location => Color::Blue,
        Tone::Status | Tone::Warn => Color::Yellow,
        Tone::Ok => Color::Green,
        Tone::Alert => Color::Red,
        Tone::Identity => Color::Gray,
    }
}

/// A segment is a block of its tone's color with the text in the terminal's background color.
fn block_style(theme: &Theme, tone: Tone) -> Style {
    if theme.color {
        Style::new()
            .fg(tone_color(tone))
            .add_modifier(Modifier::REVERSED)
    } else if tone == Tone::Alert {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::REVERSED)
    }
}

/// The text of a segment with a space of block on each side.
fn block(seg: &Segment, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {} ", sanitize(&seg.text)),
        block_style(theme, seg.tone),
    )
}

/// An arrow glyph in `from`'s color on `to`'s, or on the terminal's background when `to` is none.
fn arrow(glyph: &'static str, theme: &Theme, from: Tone, to: Option<Tone>) -> Span<'static> {
    let mut style = Style::new();
    if theme.color {
        style = style.fg(tone_color(from));
        if let Some(to) = to {
            style = style.bg(tone_color(to));
        }
    }
    Span::styled(glyph, style)
}

fn left_group(segs: &[Segment], theme: &Theme, edge: SegmentEdge) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        spans.push(block(seg, theme));
        if edge == SegmentEdge::Powerline {
            let next = segs.get(i + 1).map(|s| s.tone);
            spans.push(arrow(LEFT_ARROW, theme, seg.tone, next));
        }
    }
    Line::from(spans)
}

/// One run of spans per segment, so segments can be dropped whole.
fn right_group(segs: &[Segment], theme: &Theme, edge: SegmentEdge) -> Vec<Vec<Span<'static>>> {
    let mut group = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        let mut spans = Vec::new();
        if edge == SegmentEdge::Powerline {
            let prev = i.checked_sub(1).map(|j| segs[j].tone);
            spans.push(arrow(RIGHT_ARROW, theme, seg.tone, prev));
        }
        spans.push(block(seg, theme));
        group.push(spans);
    }
    group
}

fn width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// Draws the groups on the top border row, never on the corner cells.
fn draw_top(buf: &mut Buffer, area: Rect, chrome: &Chrome, theme: &Theme, edge: SegmentEdge) {
    let avail = usize::from(area.width - 2);
    let left = left_group(&chrome.left, theme, edge);
    let right = right_group(&chrome.right, theme, edge);
    let mut keep = right.len();
    let mut right_width: usize = right.iter().map(|spans| width(spans)).sum();
    // Kept segments leave at least one border cell before the right group.
    while keep > 0 && left.width() + right_width >= avail {
        keep -= 1;
        right_width -= width(&right[keep]);
    }
    let right = Line::from(right[..keep].concat());
    // What is kept fits beside the left group, so it is at most `avail`.
    let right_width = right_width as u16;
    buf.set_line(area.x + 1, area.y, &left, area.width - 2 - right_width);
    buf.set_line(area.right() - 1 - right_width, area.y, &right, right_width);
}

/// Draws the hints and the position on the bottom border row, never on the corner cells.
fn draw_bottom(buf: &mut Buffer, area: Rect, chrome: &Chrome, theme: &Theme) {
    let avail = usize::from(area.width - 2);
    let position = Line::styled(
        format!(" {} ", sanitize(&chrome.position)),
        theme.style(Paint::Minor),
    );
    let position_width = position.width().min(avail);
    let hints: Vec<Vec<Span>> = chrome
        .hints
        .iter()
        .map(|(key, label)| {
            vec![
                Span::styled(sanitize(key), theme.style(Paint::Location)),
                Span::raw(" "),
                Span::styled(sanitize(label), theme.style(Paint::Plain)),
            ]
        })
        .collect();
    // A space on each side of the hints, the gap between two of them.
    let hints_width = |hints: &[Vec<Span>]| {
        let gaps = HINT_GAP.len() * hints.len().saturating_sub(1);
        2 + gaps + hints.iter().map(|spans| width(spans)).sum::<usize>()
    };
    let mut keep = hints.len();
    while keep > 0 && hints_width(&hints[..keep]) + position_width > avail {
        keep -= 1;
    }
    let y = area.bottom() - 1;
    if keep > 0 {
        let mut spans = vec![Span::raw(" ")];
        for (i, hint) in hints[..keep].iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(HINT_GAP));
            }
            spans.extend(hint.iter().cloned());
        }
        spans.push(Span::raw(" "));
        buf.set_line(area.x + 1, y, &Line::from(spans), area.width - 2);
    }
    // At most `avail`, by the `min` above.
    let position_width = position_width as u16;
    buf.set_line(
        area.right() - 1 - position_width,
        y,
        &position,
        position_width,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use unicode_width::UnicodeWidthStr;

    use super::*;

    fn seg(text: &str, tone: Tone) -> Segment {
        Segment {
            text: text.into(),
            tone,
        }
    }

    fn chrome() -> Chrome {
        Chrome {
            left: vec![
                seg("V2EX", Tone::Site),
                seg("python", Tone::Location),
                seg("+4 ~2", Tone::Status),
            ],
            right: vec![seg("✔ API 571/600", Tone::Ok), seg("guest", Tone::Identity)],
            hints: ["↑↓ 移动", "→ 阅读", "s 换节点", "? 说明"]
                .iter()
                .map(|h| {
                    let (key, label) = h.split_once(' ').unwrap();
                    (key.to_string(), label.to_string())
                })
                .collect(),
            position: "3/50".into(),
        }
    }

    fn draw(w: u16, h: u16, chrome: &Chrome, color: bool, edge: SegmentEdge) -> (Buffer, Rect) {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut inner = Rect::default();
        terminal
            .draw(|f| inner = draw_frame(f, f.area(), chrome, &Theme { color }, edge))
            .unwrap();
        (terminal.backend().buffer().clone(), inner)
    }

    /// The text of one row; the hidden cells behind a wide character add nothing.
    fn row(buf: &Buffer, y: u16) -> String {
        let mut out = String::new();
        let mut hidden = 0;
        for x in 0..buf.area.width {
            if hidden > 0 {
                hidden -= 1;
                continue;
            }
            let symbol = buf[(x, y)].symbol();
            hidden = symbol.width().saturating_sub(1);
            out.push_str(symbol);
        }
        out
    }

    const EDGES: [SegmentEdge; 2] = [SegmentEdge::Plain, SegmentEdge::Powerline];

    #[test]
    fn corners_and_right_border_are_intact_at_every_width() {
        for edge in EDGES {
            for color in [true, false] {
                for (w, h) in [(40, 8), (80, 24), (200, 50)] {
                    let (buf, inner) = draw(w, h, &chrome(), color, edge);
                    assert_eq!(buf[(0, 0)].symbol(), "╭");
                    assert_eq!(buf[(w - 1, 0)].symbol(), "╮");
                    assert_eq!(buf[(0, h - 1)].symbol(), "╰");
                    assert_eq!(buf[(w - 1, h - 1)].symbol(), "╯");
                    for y in 1..h - 1 {
                        assert_eq!(buf[(0, y)].symbol(), "│");
                        assert_eq!(buf[(w - 1, y)].symbol(), "│");
                    }
                    for y in [0, h - 1] {
                        assert_eq!(row(&buf, y).width(), usize::from(w));
                    }
                    assert_eq!(
                        inner,
                        Rect {
                            x: 1,
                            y: 1,
                            width: w - 2,
                            height: h - 2
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn a_narrow_frame_keeps_the_left_segments_and_the_position() {
        let (buf, _) = draw(40, 8, &chrome(), true, SegmentEdge::Plain);
        let (top, bottom) = (row(&buf, 0), row(&buf, 7));
        assert!(top.contains("V2EX") && top.contains("python") && top.contains("+4 ~2"));
        assert!(top.contains("✔ API 571/600"));
        assert!(!top.contains("guest"));
        assert!(bottom.contains("3/50"));
        assert!(bottom.contains("↑↓ 移动"));
        assert!(!bottom.contains("? 说明"));

        for edge in EDGES {
            let (buf, _) = draw(40, 8, &chrome(), true, edge);
            let (top, bottom) = (row(&buf, 0), row(&buf, 7));
            assert!(top.contains("V2EX") && top.contains("python"));
            assert!(!top.contains("guest"));
            assert!(bottom.contains("3/50"));
        }
    }

    #[test]
    fn a_wide_frame_shows_everything() {
        let (buf, _) = draw(120, 8, &chrome(), true, SegmentEdge::Plain);
        let (top, bottom) = (row(&buf, 0), row(&buf, 7));
        assert!(top.starts_with("╭ V2EX  python  +4 ~2 ─"));
        assert!(top.ends_with("─ ✔ API 571/600  guest ╮"));
        assert!(bottom.starts_with("╰ ↑↓ 移动   → 阅读   s 换节点   ? 说明 ─"));
        assert!(bottom.ends_with("─ 3/50 ╯"));

        let (buf, _) = draw(120, 8, &chrome(), true, SegmentEdge::Powerline);
        let (top, bottom) = (row(&buf, 0), row(&buf, 7));
        assert!(top.starts_with("╭ V2EX \u{e0b0} python \u{e0b0} +4 ~2 \u{e0b0}─"));
        assert!(top.ends_with("─\u{e0b2} ✔ API 571/600 \u{e0b2} guest ╮"));
        assert!(bottom.contains("↑↓ 移动") && bottom.contains("? 说明"));
    }

    #[test]
    fn plain_edges_use_no_powerline_glyphs_and_powerline_does() {
        let has = |buf: &Buffer, range: std::ops::RangeInclusive<char>| {
            buf.content()
                .iter()
                .any(|cell| cell.symbol().chars().any(|c| range.contains(&c)))
        };
        let (plain, _) = draw(120, 8, &chrome(), true, SegmentEdge::Plain);
        assert!(!has(&plain, '\u{e0b0}'..='\u{e0b3}'));
        let (powerline, _) = draw(120, 8, &chrome(), true, SegmentEdge::Powerline);
        assert!(has(&powerline, '\u{e0b0}'..='\u{e0b0}'));
    }

    #[test]
    fn wide_text_in_a_segment_never_overwrites_the_corner() {
        let wide = "节".repeat(30);
        let mut left = chrome();
        left.left = vec![seg(&wide, Tone::Location)];
        let mut right = chrome();
        right.left.clear();
        right.right = vec![seg(&wide, Tone::Location)];
        let mut position = chrome();
        position.position = wide.clone();
        let mut hint = chrome();
        hint.hints = vec![("节".repeat(30), wide.clone())];

        for chrome in [&left, &right, &position, &hint] {
            for edge in EDGES {
                let (buf, _) = draw(40, 8, chrome, true, edge);
                assert_eq!(buf[(0, 0)].symbol(), "╭");
                assert_eq!(buf[(39, 0)].symbol(), "╮");
                assert_eq!(buf[(0, 7)].symbol(), "╰");
                assert_eq!(buf[(39, 7)].symbol(), "╯");
                for y in [0, 7] {
                    assert_eq!(row(&buf, y).width(), 40);
                }
            }
        }

        // The left group and the position are cut, not dropped.
        let (buf, _) = draw(40, 8, &left, true, SegmentEdge::Plain);
        assert!(row(&buf, 0).starts_with("╭ 节节"));
        let (buf, _) = draw(40, 8, &position, true, SegmentEdge::Plain);
        assert!(row(&buf, 7).starts_with("╰ 节节"));
    }

    #[test]
    fn text_in_the_borders_loses_control_characters_and_stays_on_one_line() {
        let mut c = chrome();
        c.left = vec![seg("V2\nEX\x1b[0m", Tone::Site)];
        let (buf, _) = draw(80, 8, &c, true, SegmentEdge::Plain);
        assert!(row(&buf, 0).starts_with("╭ V2EX[0m ─"));
    }

    #[test]
    fn too_small_draws_the_notice_without_panicking() {
        for (w, h) in [(10, 2), (1, 1), (39, 8), (40, 7), (0, 0)] {
            let area = Rect::new(0, 0, w, h);
            assert!(!fits(area));
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw_too_small(f, f.area())).unwrap();
        }
        assert!(fits(Rect::new(0, 0, 40, 8)));
        assert!(fits(Rect::new(0, 0, 200, 50)));

        let mut terminal = Terminal::new(TestBackend::new(39, 8)).unwrap();
        terminal.draw(|f| draw_too_small(f, f.area())).unwrap();
        assert!(row(terminal.backend().buffer(), 0).contains("终端太小，至少需要 40×8"));
    }

    #[test]
    fn the_too_small_notice_stays_on_one_row() {
        for (w, first) in [(10, "终端太小，"), (9, "终端太小 "), (1, " ")] {
            let mut terminal = Terminal::new(TestBackend::new(w, 3)).unwrap();
            terminal.draw(|f| draw_too_small(f, f.area())).unwrap();
            let buf = terminal.backend().buffer();
            assert_eq!(row(buf, 0), first, "width {w}");
            for y in 1..3 {
                assert_eq!(row(buf, y).trim(), "", "width {w}, row {y}");
            }
        }
    }

    #[test]
    fn the_top_groups_keep_a_border_cell_between_them() {
        let a = "a".repeat(20);
        let mut c = chrome();
        c.left = vec![seg("V2EX", Tone::Site)];
        // 6 + 22 + 9 columns of the 38 between the corners: one border cell is left.
        c.right = vec![seg(&a, Tone::Ok), seg("bbbbbbb", Tone::Identity)];
        let (buf, _) = draw(40, 8, &c, true, SegmentEdge::Plain);
        assert_eq!(row(&buf, 0), format!("╭ V2EX ─ {a}  bbbbbbb ╮"));

        // 6 + 22 + 10: no border cell would be left, so the last segment goes.
        c.right[1] = seg("bbbbbbbb", Tone::Identity);
        let (buf, _) = draw(40, 8, &c, true, SegmentEdge::Plain);
        assert_eq!(row(&buf, 0), format!("╭ V2EX {} {a} ╮", "─".repeat(10)));
    }

    #[test]
    fn drawing_the_frame_in_a_tiny_area_does_not_panic() {
        for w in 0..=6 {
            for h in 0..=6 {
                for edge in EDGES {
                    draw(w, h, &chrome(), true, edge);
                }
            }
        }
    }
}
