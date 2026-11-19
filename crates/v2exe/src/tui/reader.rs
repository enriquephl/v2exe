//! The reader of one topic and its replies.

use chrono::{DateTime, FixedOffset, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line as TextLine, Span};
use v2ex_rs::{Reply, Topic};

use super::app::{App, Cmd, Screen, hint, segment};
use super::frame::{Chrome, Tone, draw_frame};
use super::help::HelpScreen;
use super::prompt::{Prompt, PromptKey};
use super::theme::{Paint, Theme};
use super::topics::fit;
use crate::render::{Line, LineKind, Role, Seg, display_width, render_html, sanitize, wrap};
use crate::store::config::SegmentEdge;

/// The widest the body and the replies are laid out, in columns.
const MAX_WIDTH: usize = 100;

/// Columns a reply's content is indented by.
const REPLY_INDENT: usize = 4;

/// The label of the search prompt.
const SEARCH: &str = "查找";

/// Between the parts of the line under the title.
const DOT: &str = " · ";

/// One laid-out line of the reader: runs of text, each with its paint.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RLine {
    pub spans: Vec<(String, Paint)>,
}

impl RLine {
    /// The spans' text joined together.
    pub fn text(&self) -> String {
        self.spans.iter().map(|(text, _)| text.as_str()).collect()
    }
}

/// A topic and its replies as lines for a given width.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub lines: Vec<RLine>,
    /// `floors[n - 1]` is the number of the first line of the block of floor `n`.
    pub floors: Vec<usize>,
}

/// The title, a line of the author, the time and the reply count, a blank line and the body,
/// then a block per reply. `width` is the columns there are; the body is wrapped to
/// `min(width, 100)`.
pub fn layout(topic: &Topic, replies: &[Reply], width: usize, tz: &FixedOffset) -> Layout {
    let column = width.min(MAX_WIDTH);
    let title = Line {
        kind: LineKind::Text,
        segs: vec![Seg {
            text: sanitize(&topic.title),
            role: Role::Bold,
        }],
    };
    let mut lines: Vec<RLine> = wrap(&[title], width)
        .iter()
        .map(|line| spans(Vec::new(), line))
        .collect();
    lines.push(meta(topic, width, tz));

    let body = topic
        .content_html
        .as_deref()
        .map(render_html)
        .unwrap_or_default();
    if !body.is_empty() || !replies.is_empty() {
        lines.push(RLine::default());
    }
    lines.extend(block(&body, 0, column));
    if !body.is_empty() && !replies.is_empty() {
        lines.push(RLine::default());
    }

    let mut floors = Vec::new();
    for reply in replies {
        floors.push(lines.len());
        lines.push(reply_header(reply, &topic.author.username, width, tz));
        lines.extend(block(
            &render_html(&reply.content_html),
            REPLY_INDENT,
            column,
        ));
    }
    Layout { lines, floors }
}

/// `作者 · YYYY-MM-DD HH:MM · N 回复`, with the author cut to fit `width`.
fn meta(topic: &Topic, width: usize, tz: &FixedOffset) -> RLine {
    let time = topic
        .created
        .with_timezone(tz)
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let count = format!("{} 回复", topic.replies);
    let rest = display_width(&time) + display_width(&count) + 2 * display_width(DOT);
    let author = fit(
        &sanitize(&topic.author.username),
        width.saturating_sub(rest),
    );
    RLine {
        spans: vec![
            (author, Paint::Person),
            (DOT.into(), Paint::Minor),
            (time, Paint::Minor),
            (DOT.into(), Paint::Minor),
            (count, Paint::Minor),
        ],
    }
}

/// `#楼层号 名字`, ` 楼主` when the reply's author is `op`, and the time `MM/DD HH:MM` at the
/// right of `width` columns. The name is cut to leave a space before the time.
fn reply_header(reply: &Reply, op: &str, width: usize, tz: &FixedOffset) -> RLine {
    let floor = format!("#{}", reply.floor);
    let owner = (reply.author.username == op).then_some(" 楼主");
    let time = reply
        .created
        .with_timezone(tz)
        .format("%m/%d %H:%M")
        .to_string();
    let fixed = display_width(&floor) + 1 + owner.map_or(0, display_width) + 1;
    let room = width.saturating_sub(fixed + display_width(&time));
    let name = fit(&sanitize(&reply.author.username), room);
    let left = fixed - 1 + display_width(&name);
    let pad = width.saturating_sub(left + display_width(&time));

    let mut spans = vec![
        (floor, Paint::Minor),
        (" ".into(), Paint::Plain),
        (name, Paint::Person),
    ];
    if let Some(owner) = owner {
        spans.push((owner.into(), Paint::Op));
    }
    spans.push((" ".repeat(pad), Paint::Plain));
    spans.push((time, Paint::Minor));
    RLine { spans }
}

/// Rendered lines wrapped so that each, after `indent` spaces and the bar of a code or a quote
/// line, is at most `column` columns. A blank text line stays empty.
fn block(lines: &[Line], indent: usize, column: usize) -> Vec<RLine> {
    let mut out = Vec::new();
    for line in lines {
        if line.kind == LineKind::Text && line.segs.is_empty() {
            out.push(RLine::default());
            continue;
        }
        let mut prefix = Vec::new();
        if indent > 0 {
            prefix.push((" ".repeat(indent), Paint::Plain));
        }
        match line.kind {
            LineKind::Text => {}
            LineKind::Code => {
                prefix.push(("  ".into(), Paint::Plain));
                prefix.push(("▎".into(), Paint::CodeBar));
            }
            LineKind::Quote => prefix.push(("  │ ".into(), Paint::Quote)),
        }
        let prefix_width: usize = prefix.iter().map(|(text, _)| display_width(text)).sum();
        let pieces = wrap(
            std::slice::from_ref(line),
            column.saturating_sub(prefix_width),
        );
        out.extend(pieces.iter().map(|piece| spans(prefix.clone(), piece)));
    }
    out
}

/// `prefix`, then the segments of `line` in their paints.
fn spans(mut prefix: Vec<(String, Paint)>, line: &Line) -> RLine {
    prefix.extend(line.segs.iter().map(|seg| {
        let paint = match seg.role {
            Role::Plain => Paint::Plain,
            Role::Bold => Paint::Bold,
            Role::Link => Paint::Link,
            Role::Mention => Paint::Mention,
            Role::Code => Paint::Code,
            Role::Image => Paint::Image,
        };
        (seg.text.clone(), paint)
    }));
    RLine { spans: prefix }
}

pub struct ReaderScreen {
    /// The topic's id.
    pub id: u64,
    /// The ticket of the request whose result the screen is waiting for.
    pub pending: Option<u64>,
    pub topic: Option<Topic>,
    pub replies: Vec<Reply>,
    pub layout: Layout,
    /// The line at the top of the page.
    pub scroll: usize,
    /// How many more replies the topic had when opened than when last recorded.
    pub new_replies: u32,
    /// Digits typed before `Enter`, for jumping to a floor.
    pub typed: String,
    /// The search being typed.
    pub prompt: Option<Prompt>,
    /// The text searched for, ignoring case.
    pub search: Option<String>,
}

impl ReaderScreen {
    pub fn new(id: u64, ticket: u64, new_replies: u32) -> ReaderScreen {
        ReaderScreen {
            id,
            pending: Some(ticket),
            topic: None,
            replies: Vec::new(),
            layout: Layout::default(),
            scroll: 0,
            new_replies,
            typed: String::new(),
            prompt: None,
            search: None,
        }
    }

    /// Lays the topic out again for `width` columns, once it has arrived, and keeps `scroll`
    /// in range for a page of `page` rows.
    pub(super) fn reflow(&mut self, width: usize, page: usize, tz: &FixedOffset) {
        if let Some(topic) = &self.topic {
            self.layout = layout(topic, &self.replies, width, tz);
        }
        self.scroll_to(self.scroll, page);
    }

    /// The last line `scroll` can be: the one that puts the last line at the bottom of the page.
    fn max_scroll(&self, page: usize) -> usize {
        self.layout.lines.len().saturating_sub(page)
    }

    fn scroll_to(&mut self, line: usize, page: usize) {
        self.scroll = line.min(self.max_scroll(page));
    }

    /// The numbers of the lines that contain the search, ignoring case.
    fn matches(&self) -> Vec<usize> {
        let Some(search) = &self.search else {
            return Vec::new();
        };
        let search = search.to_lowercase();
        self.layout
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.text().to_lowercase().contains(&search))
            .map(|(i, _)| i)
            .collect()
    }

    /// A key while the search is typed. `Enter` searches from the top of the page down (an
    /// empty text is no search); `Esc` and `←` keep the search there was.
    fn search_key(&mut self, key: KeyEvent, page: usize) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match prompt.key(key) {
            PromptKey::Edited | PromptKey::Complete => {}
            PromptKey::Cancel => self.prompt = None,
            PromptKey::Submit => {
                let text = std::mem::take(&mut prompt.text);
                self.prompt = None;
                self.search = Some(text).filter(|text| !text.is_empty());
                if let Some(line) = self.matches().into_iter().find(|&i| i >= self.scroll) {
                    self.scroll_to(line, page);
                }
            }
        }
    }

    /// Where the page ends as a share of the lines, such as `62%`; `100%` when they fit on one
    /// page.
    fn position(&self, page: usize) -> String {
        let len = self.layout.lines.len();
        if len <= page {
            return "100%".to_string();
        }
        format!("{}%", ((self.scroll + page) * 100 / len).min(100))
    }
}

impl App {
    /// A key while the reader is on top.
    pub(super) fn reader_key(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Vec<Cmd> {
        let page = self.inner_height();
        let Some(Screen::Reader(screen)) = self.stack.last_mut() else {
            return Vec::new();
        };
        if screen.prompt.is_some() {
            screen.search_key(key, page);
            return Vec::new();
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if let KeyCode::Char(c @ '0'..='9') = key.code
            && plain
        {
            screen.typed.push(c);
            return Vec::new();
        }
        let typed = std::mem::take(&mut screen.typed);
        let (scroll, last) = (screen.scroll, screen.max_scroll(page));
        // Space goes on from the end of a shown thread only, so it never skips a topic that is
        // still loading or failed to load.
        let shown = screen.pending.is_none() && screen.topic.is_some();
        let target = match key.code {
            KeyCode::Enter => {
                let line = typed
                    .parse::<usize>()
                    .ok()
                    .and_then(|floor| floor.checked_sub(1))
                    .and_then(|i| screen.layout.floors.get(i).copied());
                match line {
                    Some(line) => line,
                    None => return Vec::new(),
                }
            }
            KeyCode::Up => scroll.saturating_sub(1),
            KeyCode::Down => scroll + 1,
            KeyCode::PageUp => scroll.saturating_sub(page),
            KeyCode::PageDown => scroll + page,
            KeyCode::Home => 0,
            KeyCode::End => last,
            KeyCode::Left => {
                self.stack.pop();
                return Vec::new();
            }
            KeyCode::Char('b') if ctrl => scroll.saturating_sub(page),
            KeyCode::Char('f') if ctrl => scroll + page,
            KeyCode::Char(c) if plain => match c {
                'k' => scroll.saturating_sub(1),
                'j' => scroll + 1,
                ' ' if scroll == last && shown => return self.walk(true, now),
                ' ' => scroll + page,
                'g' => 0,
                'G' => last,
                'n' => match screen.matches().into_iter().find(|&i| i > scroll) {
                    Some(line) => line,
                    None => return Vec::new(),
                },
                'N' => match screen.matches().into_iter().rev().find(|&i| i < scroll) {
                    Some(line) => line,
                    None => return Vec::new(),
                },
                '/' => {
                    screen.prompt = Some(Prompt {
                        label: SEARCH,
                        text: String::new(),
                    });
                    return Vec::new();
                }
                ']' => return self.walk(true, now),
                '[' => return self.walk(false, now),
                'o' => {
                    return match &screen.topic {
                        Some(topic) => vec![Cmd::OpenUrl(topic.url.clone())],
                        None => Vec::new(),
                    };
                }
                'r' => {
                    let id = screen.id;
                    let ticket = self.ticket();
                    if let Some(Screen::Reader(screen)) = self.stack.last_mut() {
                        screen.pending = Some(ticket);
                    }
                    return vec![Cmd::LoadThread {
                        ticket,
                        id,
                        refresh: true,
                    }];
                }
                'q' | 'h' => {
                    self.stack.pop();
                    return Vec::new();
                }
                '?' => {
                    self.stack.push(Screen::Help(HelpScreen::default()));
                    return Vec::new();
                }
                _ => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        screen.scroll_to(target, page);
        Vec::new()
    }

    /// When a topic list is right under the reader on top, opens the topic after (`forward`) or
    /// before the reader's among the list's rows in place of the reader, and moves the list's
    /// cursor to it. Nothing happens at either end of the rows or without such a list.
    fn walk(&mut self, forward: bool, now: DateTime<Utc>) -> Vec<Cmd> {
        let page = self.topics_page();
        let [.., Screen::Topics(list), Screen::Reader(reader)] = self.stack.as_mut_slice() else {
            return Vec::new();
        };
        let rows = list.rows();
        let at = rows
            .iter()
            .position(|&(_, topic)| topic.id == reader.id)
            .unwrap_or(list.nav.cursor);
        let target = if forward {
            Some(at + 1)
        } else {
            at.checked_sub(1)
        };
        let Some(target) = target else {
            return Vec::new();
        };
        let Some(&(_, topic)) = rows.get(target) else {
            return Vec::new();
        };
        let (id, replies, len) = (topic.id, topic.replies, rows.len());
        list.nav.cursor = target;
        list.nav.clamp(len, page);
        self.stack.pop();
        self.open_from_list(id, replies, now)
    }
}

/// Draws the reader.
pub(super) fn draw(
    app: &App,
    screen: &ReaderScreen,
    theme: &Theme,
    edge: SegmentEdge,
    now: DateTime<Utc>,
    frame: &mut Frame,
) {
    let mut left = vec![segment("V2EX", Tone::Site)];
    if let Some(node) = screen.topic.as_ref().and_then(|topic| topic.node.as_ref()) {
        left.push(segment(node.title.as_str(), Tone::Location));
    }
    if screen.pending.is_some() {
        left.push(segment("…", Tone::Status));
    } else if screen.new_replies > 0 {
        left.push(segment(
            format!("~ {} 则新回复", screen.new_replies),
            Tone::Status,
        ));
    }
    let chrome = Chrome {
        left,
        right: app.right_segments(now),
        hints: hints(screen),
        position: screen.position(app.inner_height()),
    };
    let inner = draw_frame(frame, frame.area(), &chrome, theme, edge);
    draw_lines(frame.buffer_mut(), inner, screen, theme);
}

/// The search prompt, the floor being typed, or the screen's keys.
fn hints(screen: &ReaderScreen) -> Vec<(String, String)> {
    if let Some(prompt) = &screen.prompt {
        return vec![prompt.hint()];
    }
    if !screen.typed.is_empty() {
        return vec![hint("Enter", &format!("跳到 {} 楼", screen.typed))];
    }
    vec![
        hint("↑↓", "滚动"),
        hint("空格", "翻页"),
        hint("]", "下一篇"),
        hint("←", "返回"),
    ]
}

/// The lines from `scroll` down, with a column of space inside each side of the border. A line
/// that contains the search is reversed across the row.
fn draw_lines(buf: &mut Buffer, inner: Rect, screen: &ReaderScreen, theme: &Theme) {
    let area = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    let matches = screen.matches();
    let shown = screen.layout.lines.iter().enumerate().skip(screen.scroll);
    for (y, (i, line)) in (area.top()..area.bottom()).zip(shown) {
        let spans: Vec<Span> = line
            .spans
            .iter()
            .map(|(text, paint)| Span::styled(text.as_str(), theme.style(*paint)))
            .collect();
        buf.set_line(area.x, y, &TextLine::from(spans), area.width);
        if matches.binary_search(&i).is_ok() {
            let row = Rect {
                y,
                height: 1,
                ..area
            };
            buf.set_style(row, Style::new().add_modifier(Modifier::REVERSED));
        }
    }
}

#[cfg(test)]
mod tests {
    use v2ex_rs::Topic;

    use super::*;
    use crate::render::display_width;
    use crate::tui::app::testing::{reply, topic, tz};

    fn texts(laid: &Layout) -> Vec<String> {
        laid.lines.iter().map(RLine::text).collect()
    }

    /// 「示例标题」 by alice with `html` as its body.
    fn with_body(html: &str, replies: u32) -> Topic {
        let mut t = topic(1, "示例标题", "alice", replies);
        t.content_html = Some(html.into());
        t
    }

    #[test]
    fn layout_puts_title_meta_body_then_one_block_per_reply() {
        let t = with_body("<p>第一段</p><pre>x = 1</pre>", 2);
        let replies = [reply(1, "bob", "第一则"), reply(2, "alice", "第二则")];
        let width = 60;
        let laid = layout(&t, &replies, width, &tz());
        let lines = texts(&laid);
        assert_eq!(lines.len(), 11, "{lines:#?}");
        assert_eq!(
            lines[..7],
            [
                "示例标题",
                "alice · 2026-11-19 09:00 · 2 回复",
                "",
                "第一段",
                "",
                "  ▎x = 1",
                ""
            ]
        );
        assert!(
            lines[7].starts_with("#1 bob ") && lines[7].ends_with(" 11/19 10:00"),
            "{:?}",
            lines[7]
        );
        assert!(!lines[7].contains("楼主"));
        assert_eq!(display_width(&lines[7]), width);
        assert_eq!(lines[8], "    第一则");
        assert!(lines[9].starts_with("#2 alice 楼主 "), "{:?}", lines[9]);
        assert!(lines[9].ends_with(" 11/19 10:00"), "{:?}", lines[9]);
        assert_eq!(display_width(&lines[9]), width);
        assert_eq!(lines[10], "    第二则");
        assert_eq!(laid.floors, [7, 9]);
    }

    #[test]
    fn quotes_and_images_keep_their_marks_inside_a_reply() {
        let t = with_body("<blockquote>引用</blockquote>", 1);
        let html = "<blockquote>引用</blockquote><img src=\"https://i.example.com/a.png\">";
        let lines = texts(&layout(&t, &[reply(1, "bob", html)], 60, &tz()));
        assert_eq!(lines[3], "  │ 引用");
        assert_eq!(
            lines[6..],
            ["      │ 引用", "", "    [图片] https://i.example.com/a.png"]
        );
    }

    #[test]
    fn no_line_is_wider_than_the_width_and_body_stops_at_100_columns() {
        let url = format!("https://example.com/{}", "a".repeat(280));
        assert_eq!(url.chars().count(), 300);
        let t = with_body(
            &format!(
                "<p>{}</p><p><a href=\"{url}\">{url}</a></p>",
                "字".repeat(500)
            ),
            1,
        );
        let replies = [reply(
            1,
            "bob",
            &format!(
                "<pre>{}</pre><blockquote>{}</blockquote>",
                "x".repeat(400),
                "引".repeat(200)
            ),
        )];
        for width in [40, 41, 80, 160] {
            for line in texts(&layout(&t, &replies, width, &tz())) {
                assert!(display_width(&line) <= width, "{width}: {line:?}");
            }
        }

        let laid = layout(&t, &replies, 160, &tz());
        let lines = texts(&laid);
        // Past the title and the meta line, every line but a reply's first is body or reply.
        for (i, line) in lines.iter().enumerate().skip(2) {
            if !laid.floors.contains(&i) {
                assert!(display_width(line) <= 100, "{line:?}");
            }
        }
        assert!(lines.iter().any(|line| display_width(line) == 100));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("x".repeat(90).as_str()))
        );
    }

    #[test]
    fn a_topic_without_body_or_replies_has_no_empty_blocks() {
        let bare = topic(1, "示例标题", "alice", 0);
        let laid = layout(&bare, &[], 60, &tz());
        assert_eq!(
            texts(&laid),
            ["示例标题", "alice · 2026-11-19 09:00 · 0 回复"]
        );
        assert!(laid.floors.is_empty());

        let laid = layout(&bare, &[reply(1, "bob", "第一则")], 60, &tz());
        let lines = texts(&laid);
        assert_eq!(lines.len(), 5, "{lines:#?}");
        assert_eq!(lines[2], "");
        assert!(lines[3].starts_with("#1 bob"), "{:?}", lines[3]);
        assert_eq!(lines[4], "    第一则");
        assert_eq!(laid.floors, [3]);

        // A body without replies is the end.
        let laid = layout(&with_body("<p>第一段</p>", 0), &[], 60, &tz());
        assert_eq!(
            texts(&laid),
            [
                "示例标题",
                "alice · 2026-11-19 09:00 · 0 回复",
                "",
                "第一段"
            ]
        );
    }

    #[test]
    fn control_characters_in_names_and_titles_are_dropped() {
        let t = topic(1, "a\x1b[2Jb", "al\u{9b}ice", 2);
        let replies = [reply(1, "al\u{9b}ice", "第一则"), reply(2, "b\x07ob", "x")];
        let lines = texts(&layout(&t, &replies, 60, &tz()));
        for line in &lines {
            assert!(!line.chars().any(char::is_control), "{line:?}");
        }
        assert_eq!(lines[0], "a[2Jb");
        assert!(lines[1].starts_with("alice · "), "{:?}", lines[1]);
        assert!(lines[3].starts_with("#1 alice 楼主"), "{:?}", lines[3]);
        assert!(lines[5].starts_with("#2 bob "), "{:?}", lines[5]);
    }
}
