//! The topic list of one listing, and the goto prompt that opens a listing by name.

use chrono::{DateTime, FixedOffset, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;
use v2ex_rs::{Listing, Node, Topic};

use super::app::{App, Cmd, Screen, ask_directory, hint, segment};
use super::frame::{Chrome, Tone, draw_frame};
use super::help::HelpScreen;
use super::list::{ListNav, Nav};
use super::prompt::{Prompt, PromptKey};
use super::theme::{Paint, Theme};
use crate::render::sanitize;
use crate::store::config::SegmentEdge;
use crate::store::favorites::{TABS, listing_title};
use crate::store::read_state::{Mark, ReadState};

/// The label of the goto prompt, which tells it apart from a filter prompt.
pub(super) const GOTO: &str = "节点";

/// The label of the title filter prompt.
const FILTER: &str = "找标题";

/// The most goto candidates offered.
const MAX_COMPLETIONS: usize = 8;

/// Columns before a topic's title and its second row: the cursor and the mark.
const INDENT: u16 = 4;

pub struct TopicsScreen {
    pub listing: Listing,
    /// The listing's display name.
    pub title: String,
    pub topics: Vec<Topic>,
    /// The ticket of the request whose result the screen is waiting for.
    pub pending: Option<u64>,
    pub nav: ListNav,
    /// Topics are kept when their title contains this, ignoring case.
    pub filter: Option<String>,
    /// Topics are kept when this user posted them.
    pub author: Option<String>,
    /// The title filter or the goto input being typed.
    pub prompt: Option<Prompt>,
}

impl TopicsScreen {
    pub fn new(listing: Listing, title: String, ticket: u64) -> TopicsScreen {
        TopicsScreen {
            listing,
            title,
            topics: Vec::new(),
            pending: Some(ticket),
            nav: ListNav::default(),
            filter: None,
            author: None,
            prompt: None,
        }
    }

    /// The topics the filters keep, as (number, topic). The number is the position in the
    /// unfiltered list, counting from 1.
    pub fn rows(&self) -> Vec<(usize, &Topic)> {
        let filter = self.filter.as_deref().map(str::to_lowercase);
        self.topics
            .iter()
            .enumerate()
            .map(|(i, topic)| (i + 1, topic))
            .filter(|(_, topic)| {
                filter
                    .as_ref()
                    .is_none_or(|filter| topic.title.to_lowercase().contains(filter))
                    && self
                        .author
                        .as_ref()
                        .is_none_or(|author| topic.author.username == *author)
            })
            .collect()
    }

    /// (unread, with new replies since read), over every topic whatever the filters.
    pub fn counts(&self, read: &ReadState) -> (usize, usize) {
        let mut counts = (0, 0);
        for topic in &self.topics {
            match read.mark(topic.id, topic.replies) {
                Mark::Unread => counts.0 += 1,
                Mark::Updated(_) => counts.1 += 1,
                Mark::Read => {}
            }
        }
        counts
    }

    /// The topic under the cursor.
    fn current(&self) -> Option<&Topic> {
        self.rows().get(self.nav.cursor).map(|&(_, topic)| topic)
    }

    /// Sets the title filter (an empty one is none) and the author filter, and keeps the cursor
    /// on the same topic when it is still shown, else puts it on the first row.
    fn set_filters(&mut self, filter: Option<String>, author: Option<String>, page: usize) {
        let current = self.rows().get(self.nav.cursor).map(|&(number, _)| number);
        self.filter = filter.filter(|filter| !filter.is_empty());
        self.author = author;
        let rows = self.rows();
        let cursor = current
            .and_then(|number| rows.iter().position(|&(n, _)| n == number))
            .unwrap_or(0);
        let len = rows.len();
        self.nav.cursor = cursor;
        self.nav.clamp(len, page);
    }

    /// A key while the title filter is typed; the rows narrow as it is typed.
    fn filter_key(&mut self, key: KeyEvent, page: usize) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match prompt.key(key) {
            PromptKey::Edited => {
                let text = prompt.text.clone();
                self.set_filters(Some(text), self.author.clone(), page);
            }
            PromptKey::Submit => self.prompt = None,
            PromptKey::Cancel => {
                self.prompt = None;
                self.set_filters(None, self.author.clone(), page);
            }
            PromptKey::Complete => {}
        }
    }
}

/// The candidates for the goto input: `hot`, `latest`, the nine tab names, then the node names
/// of the directory; those that start with `input`, ignoring case, each once, at most 8.
pub fn completions(input: &str, directory: Option<&[Node]>) -> Vec<String> {
    let prefix = input.to_lowercase();
    let feeds = ["hot", "latest"]
        .into_iter()
        .chain(TABS.iter().map(|&(name, _)| name));
    let nodes = directory
        .unwrap_or_default()
        .iter()
        .map(|node| node.name.as_str());
    let mut found: Vec<String> = Vec::new();
    for name in feeds.chain(nodes) {
        if found.len() == MAX_COMPLETIONS {
            break;
        }
        if name.to_lowercase().starts_with(&prefix) && !found.iter().any(|n| n == name) {
            found.push(name.to_string());
        }
    }
    found
}

/// The listing the goto input names and its display name. `hot`, `latest` and the tab names
/// name their feeds, ignoring case; anything else names a node, titled as in the directory when
/// it is there and by the input when not. An empty input names nothing.
pub fn resolve(input: &str, directory: Option<&[Node]>) -> Option<(Listing, String)> {
    let name = input.trim();
    if name.is_empty() {
        return None;
    }
    let lower = name.to_lowercase();
    let feed = match lower.as_str() {
        "hot" => Some(Listing::Hot),
        "latest" => Some(Listing::Latest),
        _ => TABS
            .iter()
            .find(|&&(tab, _)| tab == lower)
            .map(|&(tab, _)| Listing::Tab(tab.to_string())),
    };
    if let Some(listing) = feed {
        let title = listing_title(&listing).expect("every feed and tab has a title");
        return Some((listing, title.to_string()));
    }
    let known = directory
        .unwrap_or_default()
        .iter()
        .find(|node| node.name.to_lowercase() == lower);
    Some(match known {
        Some(node) => (Listing::Node(node.name.clone()), node.title.clone()),
        None => (Listing::Node(name.to_string()), name.to_string()),
    })
}

/// The goto prompt, then each candidate as a hint of its own, so that as many as fit are shown.
pub(super) fn goto_hints(prompt: &Prompt, directory: Option<&[Node]>) -> Vec<(String, String)> {
    let mut hints = vec![prompt.hint()];
    hints.extend(
        completions(&prompt.text, directory)
            .into_iter()
            .map(|name| (name, String::new())),
    );
    hints
}

impl App {
    /// A key while the topic list is on top.
    pub(super) fn topics_key(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Vec<Cmd> {
        let page = self.topics_page();
        let Some(Screen::Topics(screen)) = self.stack.last_mut() else {
            return Vec::new();
        };
        if let Some(prompt) = &screen.prompt {
            if prompt.label == GOTO {
                return self.goto_key(key);
            }
            screen.filter_key(key, page);
            return Vec::new();
        }
        let len = screen.rows().len();
        match screen.nav.key(key, len, page) {
            Nav::Handled => Vec::new(),
            Nav::Jump(number) => {
                let found = screen.rows().iter().position(|&(n, _)| n == number);
                if let Some(i) = found {
                    screen.nav.cursor = i;
                    screen.nav.clamp(len, page);
                }
                Vec::new()
            }
            Nav::Open => match screen.current().map(|t| (t.id, t.replies)) {
                Some((id, replies)) => self.open_from_list(id, replies),
                None => Vec::new(),
            },
            Nav::Back => {
                if screen.filter.is_some() || screen.author.is_some() {
                    screen.set_filters(None, None, page);
                } else {
                    self.stack.pop();
                }
                Vec::new()
            }
            Nav::Pass if key.code == KeyCode::Char('?') => {
                self.stack.push(Screen::Help(HelpScreen::default()));
                Vec::new()
            }
            Nav::Pass => self.topics_command(key, page, now),
        }
    }

    /// The keys of the topic list that are not list keys: `r`, `a`, `v`, `o`, `/` and `s`.
    fn topics_command(&mut self, key: KeyEvent, page: usize, now: DateTime<Utc>) -> Vec<Cmd> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Vec::new();
        }
        if key.code == KeyCode::Char('r') {
            let ticket = self.ticket();
            let Some(Screen::Topics(screen)) = self.stack.last_mut() else {
                return Vec::new();
            };
            screen.pending = Some(ticket);
            return vec![Cmd::LoadTopics {
                ticket,
                listing: screen.listing.clone(),
                refresh: true,
            }];
        }
        let Some(Screen::Topics(screen)) = self.stack.last_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Char('a') => {
                let author = match (&screen.author, screen.current()) {
                    (Some(_), _) => None,
                    (None, Some(topic)) => Some(topic.author.username.clone()),
                    (None, None) => return Vec::new(),
                };
                screen.set_filters(screen.filter.clone(), author, page);
                Vec::new()
            }
            KeyCode::Char('v') => {
                let Some((id, replies)) = screen.current().map(|t| (t.id, t.replies)) else {
                    return Vec::new();
                };
                if self.read.mark(id, replies) == Mark::Unread {
                    self.read.record(id, replies, now);
                } else {
                    self.read.forget(id);
                }
                vec![Cmd::SaveRead]
            }
            KeyCode::Char('o') => match screen.current() {
                Some(topic) => vec![Cmd::OpenUrl(topic.url.clone())],
                None => Vec::new(),
            },
            KeyCode::Char('/') => {
                screen.prompt = Some(Prompt {
                    label: FILTER,
                    text: screen.filter.clone().unwrap_or_default(),
                });
                Vec::new()
            }
            KeyCode::Char('s') => {
                screen.prompt = Some(Prompt {
                    label: GOTO,
                    text: String::new(),
                });
                ask_directory(self.directory.as_deref(), &mut self.directory_loading)
            }
            _ => Vec::new(),
        }
    }

    /// A key while the goto prompt of the screen on top is open. `Tab` completes the input to
    /// the first candidate; `Enter` opens the listing it names, in place of the topic list on
    /// top or on top of the node list; `Esc` and `←` close the prompt.
    pub(super) fn goto_key(&mut self, key: KeyEvent) -> Vec<Cmd> {
        let directory = self.directory.as_deref();
        let slot = match self.stack.last_mut() {
            Some(Screen::Nodes(screen)) => &mut screen.prompt,
            Some(Screen::Topics(screen)) => &mut screen.prompt,
            _ => return Vec::new(),
        };
        let Some(prompt) = slot.as_mut() else {
            return Vec::new();
        };
        match prompt.key(key) {
            PromptKey::Edited => Vec::new(),
            PromptKey::Complete => {
                if let Some(first) = completions(&prompt.text, directory).into_iter().next() {
                    prompt.text = first;
                }
                Vec::new()
            }
            PromptKey::Cancel => {
                *slot = None;
                Vec::new()
            }
            PromptKey::Submit => {
                let target = resolve(&prompt.text, directory);
                *slot = None;
                let Some((listing, title)) = target else {
                    return Vec::new();
                };
                if let Some(Screen::Topics(_)) = self.stack.last() {
                    self.stack.pop();
                }
                self.open_listing(listing, title)
            }
        }
    }
}

/// Draws the topic list.
pub(super) fn draw(
    app: &App,
    screen: &TopicsScreen,
    theme: &Theme,
    edge: SegmentEdge,
    now: DateTime<Utc>,
    frame: &mut Frame,
) {
    let mut left = vec![
        segment("V2EX", Tone::Site),
        segment(screen.title.as_str(), Tone::Location),
    ];
    if screen.pending.is_some() {
        left.push(segment("…", Tone::Status));
    } else {
        let (fresh, changed) = screen.counts(&app.read);
        if fresh + changed > 0 {
            left.push(segment(format!("+{fresh} ~{changed}"), Tone::Status));
        }
    }
    let rows = screen.rows();
    let position = match rows.get(screen.nav.cursor) {
        Some((number, _)) => format!("{number}/{}", screen.topics.len()),
        None => "0/0".to_string(),
    };
    let chrome = Chrome {
        left,
        right: app.right_segments(now),
        hints: hints(screen, app.directory.as_deref()),
        position,
    };
    let inner = draw_frame(frame, frame.area(), &chrome, theme, edge);
    draw_rows(frame.buffer_mut(), inner, app, screen, &rows, theme);
}

/// The prompt, the number being typed, or the screen's keys with the filters in use.
fn hints(screen: &TopicsScreen, directory: Option<&[Node]>) -> Vec<(String, String)> {
    match &screen.prompt {
        Some(prompt) if prompt.label == GOTO => return goto_hints(prompt, directory),
        Some(prompt) => return vec![prompt.hint()],
        None => {}
    }
    if !screen.nav.typed.is_empty() {
        return vec![hint("Enter", &format!("跳到 {}", screen.nav.typed))];
    }
    let filter = match &screen.filter {
        Some(filter) => format!("{FILTER}：{filter}"),
        None => FILTER.to_string(),
    };
    let mut hints = vec![
        hint("↑↓", "移动"),
        hint("→", "阅读"),
        hint("s", "换节点"),
        hint("/", &filter),
    ];
    if let Some(author) = &screen.author {
        hints.push(hint("a", &format!("作者：{author}")));
    }
    hints.push(hint("?", "说明"));
    hints
}

/// Two rows per topic from the top of the page, with a column of space inside each side of the
/// border. The first: the cursor, the mark, the title, and the reply count at the right. The
/// second, under the title: the author, the day it was posted, and the last replier when known.
fn draw_rows(
    buf: &mut Buffer,
    inner: Rect,
    app: &App,
    screen: &TopicsScreen,
    rows: &[(usize, &Topic)],
    theme: &Theme,
) {
    let area = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    // Every topic starts on a row that has another below it.
    let tops = (area.top()..area.bottom().saturating_sub(1)).step_by(2);
    let page = rows.iter().enumerate().skip(screen.nav.offset);
    for (y, (i, &(_, topic))) in tops.zip(page) {
        let selected = i == screen.nav.cursor;
        let count = replies(topic.replies, theme);
        // A reply count is at most 3 columns.
        let count_width = (count.width() as u16).min(area.width);
        let mark = match app.read.mark(topic.id, topic.replies) {
            Mark::Unread => Span::styled("+ ", theme.style(Paint::Fresh)),
            Mark::Updated(_) => Span::styled("~ ", theme.style(Paint::Changed)),
            Mark::Read => Span::raw("  "),
        };
        let title_width = area.width.saturating_sub(INDENT + count_width);
        let title = fit(&sanitize(&topic.title), usize::from(title_width));
        let title_paint = if selected {
            Paint::Selected
        } else {
            Paint::Plain
        };
        let first = Line::from(vec![
            Span::styled(
                if selected { "❯ " } else { "  " },
                theme.style(Paint::Cursor),
            ),
            mark,
            Span::styled(title, theme.style(title_paint)),
        ]);
        buf.set_line(area.x, y, &first, area.width - count_width);
        buf.set_line(area.right() - count_width, y, &count, count_width);
        let second = details(topic, &app.tz, theme);
        buf.set_line(
            area.x + INDENT,
            y + 1,
            &second,
            area.width.saturating_sub(INDENT),
        );
    }
}

/// The reply count after a space, in one of three bands: 1 to 9, 10 to 99, and 「爆」 from 100;
/// nothing for none.
fn replies(n: u32, theme: &Theme) -> Line<'static> {
    let (text, paint) = match n {
        0 => (String::new(), Paint::Plain),
        1..=9 => (format!(" {n}"), Paint::RepliesFew),
        10..=99 => (format!(" {n}"), Paint::RepliesMany),
        _ => (" 爆".to_string(), Paint::RepliesHuge),
    };
    Line::styled(text, theme.style(paint))
}

/// The second row of a topic: the author, the day it was posted as `MM/DD` in `tz`, and the last
/// replier when known.
fn details(topic: &Topic, tz: &FixedOffset, theme: &Theme) -> Line<'static> {
    let day = topic.created.with_timezone(tz).format("%m/%d").to_string();
    let mut spans = vec![
        Span::styled(sanitize(&topic.author.username), theme.style(Paint::Person)),
        Span::styled(" · ", theme.style(Paint::Minor)),
        Span::styled(day, theme.style(Paint::Minor)),
    ];
    if let Some(name) = &topic.last_reply_by {
        spans.push(Span::styled(" · 最后回复 ", theme.style(Paint::Minor)));
        spans.push(Span::styled(sanitize(name), theme.style(Paint::Person)));
    }
    Line::from(spans)
}

/// `text` cut by display width to at most `width` columns, ending with `…` when cut.
pub(super) fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut end = text.len();
    while let Some(c) = text[..end].chars().next_back()
        && text[..end].width() + 1 > width
    {
        end -= c.len_utf8();
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use crossterm::event::KeyCode;
    use v2ex_rs::{Listing, Topic};

    use super::{completions, resolve};
    use crate::store::read_state::Mark;
    use crate::tui::Start;
    use crate::tui::app::testing::*;
    use crate::tui::app::{App, Cmd, Msg, Screen};

    fn numbers(app: &App) -> Vec<usize> {
        topics_screen(app)
            .rows()
            .into_iter()
            .map(|(number, _)| number)
            .collect()
    }

    fn cursor(app: &App) -> usize {
        topics_screen(app).nav.cursor
    }

    fn counts(app: &App) -> (usize, usize) {
        topics_screen(app).counts(&app.read)
    }

    fn prompt_text(app: &App) -> Option<&str> {
        topics_screen(app).prompt.as_ref().map(|p| p.text.as_str())
    }

    #[test]
    fn an_opened_topic_is_recorded_once_its_thread_loads_and_marks_follow() {
        let mut app = listing(vec![
            topic(1, "示例一", "alice", 5),
            topic(2, "示例二", "bob", 0),
            topic(3, "示例三", "carol", 12),
        ]);
        assert_eq!(counts(&app), (3, 0));

        let cmds = code(&mut app, KeyCode::Enter);
        let Some(Screen::Reader(reader)) = app.stack.last() else {
            panic!("expected the reader on top");
        };
        assert_eq!(reader.id, 1);
        assert_eq!(
            cmds,
            [Cmd::LoadThread {
                ticket: reader.pending.unwrap(),
                id: 1,
                refresh: false
            }]
        );
        assert_eq!(app.read.mark(1, 5), Mark::Unread);
        let replies = (1..=5).map(|i| reply(i, "bob", "第一则")).collect();
        let cmds = load_thread(&mut app, topic(1, "示例一", "alice", 5), replies);
        assert_eq!(cmds, [Cmd::SaveRead]);
        assert_eq!(app.read.mark(1, 5), Mark::Read);

        code(&mut app, KeyCode::Left);
        assert!(matches!(app.stack.last(), Some(Screen::Topics(_))));
        assert_eq!(counts(&app), (2, 0));

        code(&mut app, KeyCode::Char('r'));
        load_topics(
            &mut app,
            vec![
                topic(1, "示例一", "alice", 8),
                topic(2, "示例二", "bob", 0),
                topic(3, "示例三", "carol", 12),
            ],
        );
        assert_eq!(counts(&app), (2, 1));
    }

    #[test]
    fn v_toggles_read_and_a_filters_by_author() {
        let mut app = listing(vec![
            topic(1, "示例一", "alice", 5),
            topic(2, "示例二", "bob", 3),
            topic(3, "示例三", "alice", 0),
        ]);
        assert_eq!(code(&mut app, KeyCode::Char('v')), [Cmd::SaveRead]);
        assert_eq!(app.read.mark(1, 5), Mark::Read);
        assert_eq!(code(&mut app, KeyCode::Char('v')), [Cmd::SaveRead]);
        assert_eq!(app.read.mark(1, 5), Mark::Unread);
        assert_eq!(code(&mut app, KeyCode::Char('v')), [Cmd::SaveRead]);
        assert_eq!(app.read.mark(1, 5), Mark::Read);

        // A topic with new replies is not unread, so v forgets it.
        app.read.record(2, 1, now());
        code(&mut app, KeyCode::Char('j'));
        assert_eq!(code(&mut app, KeyCode::Char('v')), [Cmd::SaveRead]);
        assert_eq!(app.read.mark(2, 3), Mark::Unread);

        code(&mut app, KeyCode::End);
        assert!(code(&mut app, KeyCode::Char('a')).is_empty());
        assert_eq!(topics_screen(&app).author.as_deref(), Some("alice"));
        assert_eq!(numbers(&app), [1, 3]);
        // The cursor stays on the topic it was on.
        assert_eq!(cursor(&app), 1);

        assert!(code(&mut app, KeyCode::Char('a')).is_empty());
        assert_eq!(topics_screen(&app).author, None);
        assert_eq!(numbers(&app), [1, 2, 3]);
        assert_eq!(cursor(&app), 2);
    }

    #[test]
    fn title_filter_and_back_behave_like_the_node_list() {
        let three = || {
            vec![
                topic(1, "用 uv 管理依赖", "alice", 1),
                topic(2, "类型标注写到什么程度", "bob", 2),
                topic(3, "UV 的锁文件", "carol", 3),
            ]
        };
        let mut app = listing(three());
        chars(&mut app, "/uv");
        // The rows narrow while typing, ignoring case.
        assert_eq!(numbers(&app), [1, 3]);
        code(&mut app, KeyCode::Enter);
        assert!(topics_screen(&app).prompt.is_none());
        assert_eq!(numbers(&app), [1, 3]);

        assert!(code(&mut app, KeyCode::Left).is_empty());
        assert_eq!(topics_screen(&app).filter, None);
        assert_eq!(numbers(&app), [1, 2, 3]);
        assert_eq!(app.stack.len(), 2);
        code(&mut app, KeyCode::Left);
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));

        // ← in the prompt cancels it and removes the filter, like Esc.
        let mut app = listing(three());
        chars(&mut app, "/uv");
        code(&mut app, KeyCode::Left);
        assert!(topics_screen(&app).prompt.is_none());
        assert_eq!(topics_screen(&app).filter, None);
        assert_eq!(app.stack.len(), 2);
        chars(&mut app, "/uv");
        code(&mut app, KeyCode::Esc);
        assert_eq!(numbers(&app), [1, 2, 3]);

        // Keys that are commands elsewhere are text in the prompt.
        chars(&mut app, "/qhv");
        assert_eq!(topics_screen(&app).filter.as_deref(), Some("qhv"));
        assert_eq!(app.stack.len(), 2);
        code(&mut app, KeyCode::Esc);

        // q clears the title and author filters together, then h goes back.
        chars(&mut app, "/uv");
        code(&mut app, KeyCode::Enter);
        code(&mut app, KeyCode::Char('a'));
        assert_eq!(numbers(&app), [1]);
        code(&mut app, KeyCode::Char('q'));
        assert_eq!(
            (&topics_screen(&app).filter, &topics_screen(&app).author),
            (&None, &None)
        );
        assert_eq!(numbers(&app), [1, 2, 3]);
        code(&mut app, KeyCode::Char('h'));
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
        assert!(!app.quit);
    }

    #[test]
    fn an_empty_listing_ignores_every_key() {
        let keys = [
            KeyCode::Enter,
            KeyCode::Right,
            KeyCode::Char('l'),
            KeyCode::Char('v'),
            KeyCode::Char('a'),
            KeyCode::Char('o'),
            KeyCode::Char(' '),
            KeyCode::Char('j'),
            KeyCode::End,
            KeyCode::PageDown,
        ];
        let mut empty = listing(Vec::new());
        // A filter that matches nothing leaves an empty list too.
        let mut filtered = listing(vec![topic(1, "示例", "alice", 1)]);
        chars(&mut filtered, "/zzz");
        code(&mut filtered, KeyCode::Enter);
        for app in [&mut empty, &mut filtered] {
            for key in keys {
                assert!(code(app, key).is_empty(), "{key:?}");
                assert_eq!(app.stack.len(), 2);
                assert_eq!((cursor(app), topics_screen(app).nav.offset), (0, 0));
            }
            chars(app, "5");
            assert!(code(app, KeyCode::Enter).is_empty());
            assert!(row(&draw(app, 80, 24), 23).contains("0/0"));
            assert_eq!(app.read.mark(1, 1), Mark::Unread);
        }
    }

    #[test]
    fn o_opens_the_url_and_r_reloads_bypassing_the_cache() {
        let mut app = listing(vec![
            topic(1001, "示例一", "alice", 1),
            topic(1002, "示例二", "bob", 2),
        ]);
        assert_eq!(
            code(&mut app, KeyCode::Char('o')),
            [Cmd::OpenUrl("https://www.v2ex.com/t/1001".into())]
        );
        let cmds = code(&mut app, KeyCode::Char('r'));
        assert_eq!(
            cmds,
            [Cmd::LoadTopics {
                ticket: topics_screen(&app).pending.unwrap(),
                listing: Listing::Hot,
                refresh: true
            }]
        );
        // The topics stay while the new ones are on their way.
        assert_eq!(topics_screen(&app).topics.len(), 2);
    }

    #[test]
    fn goto_completes_resolves_and_replaces_the_listing() {
        let directory = vec![
            node("python", "Python", 50),
            node("pytorch", "PyTorch", 5),
            node("go", "Go", 30),
        ];
        assert_eq!(completions("py", Some(&directory)), ["python", "pytorch"]);
        let all = completions("", None);
        assert_eq!(all[..2], ["hot", "latest"]);
        assert_eq!(all.len(), 8);
        assert_eq!(completions("TE", None), ["tech"]);
        // A node named like a tab is listed once.
        assert_eq!(
            completions("app", Some(&[node("apple", "Apple", 9)])),
            ["apple"]
        );

        assert_eq!(resolve("hot", None), Some((Listing::Hot, "最热".into())));
        assert_eq!(
            resolve("tech", None),
            Some((Listing::Tab("tech".into()), "技术".into()))
        );
        assert_eq!(
            resolve("python", Some(&directory)),
            Some((Listing::Node("python".into()), "Python".into()))
        );
        assert_eq!(
            resolve("rust", Some(&directory)),
            Some((Listing::Node("rust".into()), "rust".into()))
        );
        assert_eq!(resolve("", None), None);
        // Case and surrounding spaces do not matter.
        assert_eq!(resolve("HOT", None), Some((Listing::Hot, "最热".into())));
        assert_eq!(
            resolve(" Python ", Some(&directory)),
            Some((Listing::Node("python".into()), "Python".into()))
        );

        // On the topic list, Tab completes and Enter replaces the listing.
        let mut app = listing(vec![topic(1, "示例", "alice", 1)]);
        load_directory(&mut app, directory.clone());
        assert!(code(&mut app, KeyCode::Char('s')).is_empty());
        chars(&mut app, "py");
        let bottom = row(&draw(&app, 80, 24), 23);
        assert!(
            bottom.contains("节点： py") && bottom.contains("python") && bottom.contains("pytorch"),
            "{bottom}"
        );
        assert!(code(&mut app, KeyCode::Tab).is_empty());
        assert_eq!(prompt_text(&app), Some("python"));
        let cmds = code(&mut app, KeyCode::Enter);
        let [Screen::Nodes(_), Screen::Topics(topics)] = app.stack.as_slice() else {
            panic!("expected [Nodes, Topics]");
        };
        assert_eq!(topics.listing, Listing::Node("python".into()));
        assert_eq!(topics.title, "Python");
        assert!(topics.prompt.is_none());
        assert_eq!(
            cmds,
            [Cmd::LoadTopics {
                ticket: topics.pending.unwrap(),
                listing: Listing::Node("python".into()),
                refresh: false
            }]
        );

        // On the node list, Enter pushes the listing; without a directory, s asks for one.
        let (mut app, _) = launch(Start::Home);
        assert_eq!(
            code(&mut app, KeyCode::Char('s')),
            [Cmd::LoadDirectory { refresh: false }]
        );
        assert!(app.directory_loading);
        assert!(row(&draw(&app, 80, 24), 23).contains("节点："));
        chars(&mut app, "latest");
        let cmds = code(&mut app, KeyCode::Enter);
        let [Screen::Nodes(_), Screen::Topics(topics)] = app.stack.as_slice() else {
            panic!("expected [Nodes, Topics]");
        };
        assert_eq!(topics.listing, Listing::Latest);
        assert_eq!(topics.title, "最新");
        assert_eq!(cmds.len(), 1);

        // While the directory is on its way, s does not ask again.
        assert!(code(&mut app, KeyCode::Char('s')).is_empty());
        // Esc and ← close the prompt without leaving the list; an empty input changes nothing.
        code(&mut app, KeyCode::Left);
        assert!(topics_screen(&app).prompt.is_none());
        code(&mut app, KeyCode::Char('s'));
        code(&mut app, KeyCode::Esc);
        assert!(topics_screen(&app).prompt.is_none());
        code(&mut app, KeyCode::Char('s'));
        assert!(code(&mut app, KeyCode::Enter).is_empty());
        assert!(topics_screen(&app).prompt.is_none());
        assert_eq!(app.stack.len(), 2);
        assert_eq!(topics_screen(&app).listing, Listing::Latest);
    }

    #[test]
    fn rows_take_two_lines_and_the_count_follows_the_height() {
        let topics: Vec<Topic> = (1..=50)
            .map(|i| topic(i, &format!("标题{i}"), &format!("user{i}"), 0))
            .collect();
        let mut app = listing(topics);
        for (h, shown) in [(24, 11), (13, 5)] {
            app.update(Msg::Resize(80, h), now());
            let buf = draw(&app, 80, h);
            assert_framed(&buf);
            let titles = (1..h - 1)
                .filter(|&y| row(&buf, y).contains("标题"))
                .count();
            assert_eq!(titles, shown);
            assert!(row(&buf, 1).contains("❯") && row(&buf, 1).contains("标题1"));
            assert!(row(&buf, 2).contains("user1") && row(&buf, 2).contains("11/19"));
            assert!(row(&buf, 3).contains("标题2") && row(&buf, 4).contains("user2"));
        }

        // A page is as many topics as are shown.
        app.update(Msg::Resize(80, 24), now());
        code(&mut app, KeyCode::PageDown);
        assert_eq!(cursor(&app), 11);
        assert!(row(&draw(&app, 80, 24), 23).contains("12/50"));
    }

    #[test]
    fn the_day_is_the_one_in_the_time_zone() {
        // 20:00 UTC on 11/18 is 04:00 on 11/19 at UTC+8.
        let mut late = topic(1, "示例", "alice", 0);
        late.created = Utc.with_ymd_and_hms(2026, 11, 18, 20, 0, 0).unwrap();
        let app = listing(vec![late]);
        let second = row(&draw(&app, 80, 24), 2);
        assert!(
            second.contains("alice · 11/19") && !second.contains("11/18"),
            "{second}"
        );
    }

    #[test]
    fn wide_titles_emoji_and_control_characters_stay_inside_the_frame() {
        let author = "n".repeat(40);
        let mut last = topic(3, "a\x1b[31mb", "e\x1b[2Jvil", 5);
        last.last_reply_by = Some("r\x1b[0me".into());
        let mut app = listing(vec![
            topic(1, &"节".repeat(60), &author, 230),
            topic(2, &"👨‍👩‍👧".repeat(30), &author, 42),
            last,
            topic(4, "", &author, 0),
        ]);
        for (w, h) in [(40, 16), (41, 16), (80, 16)] {
            app.update(Msg::Resize(w, h), now());
            let buf = draw(&app, w, h);
            assert_framed(&buf);
            let text = all_text(&buf);
            assert!(!text.contains('\x1b'));
            assert!(
                text.contains("a[31mb") && text.contains("e[2Jvil"),
                "{text}"
            );
            assert!(text.contains("r[0me"), "{text}");
            assert!(row(&buf, 1).contains("…"), "{}", row(&buf, 1));
        }
    }

    #[test]
    fn reply_counts_show_the_three_bands() {
        let app = listing(vec![
            topic(1, "零回复", "alice", 0),
            topic(2, "五回复", "bob", 5),
            topic(3, "四十二", "carol", 42),
            topic(4, "两百三十", "dave", 230),
        ]);
        let buf = draw(&app, 80, 24);
        let end = |y| row(&buf, y).trim_end_matches('│').trim_end().to_string();
        assert!(end(1).ends_with("零回复"), "{}", end(1));
        assert!(end(3).ends_with(" 5"), "{}", end(3));
        assert!(end(5).ends_with(" 42"), "{}", end(5));
        assert!(end(7).ends_with(" 爆"), "{}", end(7));
    }

    #[test]
    fn the_borders_show_the_counts_the_keys_and_the_filters() {
        let (mut app, _) = launch(Start::Listing(Listing::Node("python".into())));
        let buf = draw(&app, 80, 24);
        assert!(row(&buf, 0).contains("python") && row(&buf, 0).contains("…"));
        assert!(row(&buf, 23).contains("0/0"));

        load_topics(
            &mut app,
            vec![
                topic(1, "示例一", "alice", 5),
                topic(2, "示例二", "bob", 0),
                topic(3, "示例三", "carol", 12),
            ],
        );
        app.read.record(3, 10, now());
        let buf = draw(&app, 80, 24);
        let (top, bottom) = (row(&buf, 0), row(&buf, 23));
        assert!(top.contains("+2 ~1") && !top.contains("…"), "{top}");
        for hint in ["↑↓ 移动", "→ 阅读", "s 换节点", "/ 找标题", "? 说明", "1/3"]
        {
            assert!(bottom.contains(hint), "{hint}: {bottom}");
        }

        for id in [1, 2, 3] {
            app.read.record(id, 99, now());
        }
        let top = row(&draw(&app, 80, 24), 0);
        assert!(!top.contains('+') && !top.contains('~'), "{top}");

        chars(&mut app, "/示例");
        code(&mut app, KeyCode::Enter);
        code(&mut app, KeyCode::Char('a'));
        let bottom = row(&draw(&app, 120, 24), 23);
        assert!(
            bottom.contains("找标题：示例") && bottom.contains("作者：alice"),
            "{bottom}"
        );
    }
}
