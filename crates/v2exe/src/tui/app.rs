//! The interface as a state machine: keys and request results go in as [`Msg`], requests and
//! saves come out as [`Cmd`], and [`view`] draws one frame from the state. Nothing here touches
//! the terminal, the network, the files or the clock.

use chrono::{DateTime, FixedOffset, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use v2ex_rs::{Listing, Node, RateLimit, Reply, Topic};

use super::Start;
use super::frame::{Segment, Tone, draw_too_small, fits};
use super::help::{self, HelpScreen};
use super::nodes::{self, NodesScreen, listing_name};
use super::reader::{self, ReaderScreen};
use super::theme::Theme;
use super::topics::{self, TopicsScreen};
use crate::errors::message;
use crate::store::config::SegmentEdge;
use crate::store::favorites::{Favorites, listing_title};
use crate::store::read_state::{Mark, ReadState};

/// What happened: a key, a new terminal size, or the result of a [`Cmd`].
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "messages are handled one at a time, so their size costs nothing"
)]
pub enum Msg {
    Key(KeyEvent),
    Resize(u16, u16),
    TopicsLoaded {
        ticket: u64,
        result: v2ex_rs::Result<Vec<Topic>>,
        quota: Option<RateLimit>,
    },
    ThreadLoaded {
        ticket: u64,
        result: v2ex_rs::Result<(Topic, Vec<Reply>)>,
        quota: Option<RateLimit>,
    },
    DirectoryLoaded {
        result: v2ex_rs::Result<Vec<Node>>,
        quota: Option<RateLimit>,
    },
    /// Writing a state file or opening the browser failed; carries the reason.
    SaveFailed(String),
}

/// What the runtime is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    LoadTopics {
        ticket: u64,
        listing: Listing,
        refresh: bool,
    },
    LoadThread {
        ticket: u64,
        id: u64,
        refresh: bool,
    },
    LoadDirectory {
        refresh: bool,
    },
    OpenUrl(String),
    SaveFavorites,
    SaveRead,
}

#[expect(
    clippy::large_enum_variant,
    reason = "the stack holds a handful of screens, so their size costs nothing"
)]
pub enum Screen {
    Nodes(NodesScreen),
    Topics(TopicsScreen),
    Reader(ReaderScreen),
    Help(HelpScreen),
}

pub struct App {
    /// The screens from the bottom up; the bottom one is always [`Screen::Nodes`].
    pub stack: Vec<Screen>,
    pub favorites: Favorites,
    pub read: ReadState,
    pub directory: Option<Vec<Node>>,
    /// The node directory has been asked for and has not arrived.
    pub directory_loading: bool,
    pub quota: Option<RateLimit>,
    /// Why the last request or file write failed; cleared by the next successful request.
    pub error: Option<String>,
    /// The terminal size: (columns, rows).
    pub size: (u16, u16),
    pub tz: FixedOffset,
    pub quit: bool,
    next_ticket: u64,
}

impl App {
    /// The node list, with the screen of `start` on top of it, and the requests that screen
    /// needs.
    pub fn new(
        start: Start,
        favorites: Favorites,
        read: ReadState,
        size: (u16, u16),
        tz: FixedOffset,
    ) -> (App, Vec<Cmd>) {
        let mut app = App {
            stack: vec![Screen::Nodes(NodesScreen::default())],
            favorites,
            read,
            directory: None,
            directory_loading: false,
            quota: None,
            error: None,
            size,
            tz,
            quit: false,
            next_ticket: 0,
        };
        let cmds = match start {
            Start::Home => Vec::new(),
            Start::Listing(listing) => {
                let title = listing_title(&listing)
                    .unwrap_or_else(|| listing_name(&listing))
                    .to_string();
                app.open_listing(listing, title)
            }
            Start::Topic(id) => app.open_topic(id, 0),
        };
        (app, cmds)
    }

    pub fn update(&mut self, msg: Msg, now: DateTime<Utc>) -> Vec<Cmd> {
        match msg {
            Msg::Key(key) => self.key(key, now),
            Msg::Resize(columns, rows) => {
                self.size = (columns, rows);
                self.clamp_lists();
                let (width, page) = (self.reader_width(), self.inner_height());
                for screen in &mut self.stack {
                    match screen {
                        Screen::Reader(reader) => reader.reflow(width, page, &self.tz),
                        Screen::Help(help) => help.clamp(width, page),
                        Screen::Nodes(_) | Screen::Topics(_) => {}
                    }
                }
                Vec::new()
            }
            // The quota of every result is kept: it is the latest the site reported. The topics
            // go only to the list still waiting for this ticket; a list that was left or asked
            // again since is not.
            Msg::TopicsLoaded {
                ticket,
                result,
                quota,
            } => {
                if quota.is_some() {
                    self.quota = quota;
                }
                let waiting = self.stack.iter_mut().find_map(|screen| match screen {
                    Screen::Topics(topics) if topics.pending == Some(ticket) => Some(topics),
                    _ => None,
                });
                let Some(screen) = waiting else {
                    return Vec::new();
                };
                screen.pending = None;
                match result {
                    Ok(topics) => {
                        screen.topics = topics;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(message(&e, &self.tz)),
                }
                self.clamp_lists();
                Vec::new()
            }
            // As with the topics, only the reader still waiting for this ticket takes it. A topic
            // is recorded as read here only, once its thread has arrived, with the larger of its
            // reply count (what the lists compare against) and the number of replies that came.
            Msg::ThreadLoaded {
                ticket,
                result,
                quota,
            } => {
                if quota.is_some() {
                    self.quota = quota;
                }
                let (width, page) = (self.reader_width(), self.inner_height());
                let waiting = self.stack.iter_mut().find_map(|screen| match screen {
                    Screen::Reader(reader) if reader.pending == Some(ticket) => Some(reader),
                    _ => None,
                });
                let Some(screen) = waiting else {
                    return Vec::new();
                };
                screen.pending = None;
                match result {
                    Ok((topic, replies)) => {
                        let count = topic.replies.max(replies.len() as u32);
                        self.read.record(screen.id, count, now);
                        screen.topic = Some(topic);
                        screen.replies = replies;
                        screen.reflow(width, page, &self.tz);
                        self.error = None;
                        vec![Cmd::SaveRead]
                    }
                    Err(e) => {
                        self.error = Some(message(&e, &self.tz));
                        Vec::new()
                    }
                }
            }
            Msg::DirectoryLoaded { result, quota } => {
                if quota.is_some() {
                    self.quota = quota;
                }
                match result {
                    Ok(nodes) => {
                        self.directory = Some(nodes);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(message(&e, &self.tz)),
                }
                self.directory_loading = false;
                self.clamp_lists();
                Vec::new()
            }
            Msg::SaveFailed(reason) => {
                self.error = Some(reason);
                Vec::new()
            }
        }
    }

    /// A new ticket; each call returns one more than the last.
    pub fn ticket(&mut self) -> u64 {
        self.next_ticket += 1;
        self.next_ticket
    }

    /// The two segments at the top right: the API quota (`Warn` below a fifth left, `Alert`
    /// with the recovery time when used up, `Alert` with the reason when `error` is set) and
    /// `guest`. A quota whose recovery time has passed no longer holds and reads `API`.
    pub fn right_segments(&self, now: DateTime<Utc>) -> Vec<Segment> {
        let api = match (&self.error, &self.quota) {
            (Some(reason), _) => segment(format!("✘ {reason}"), Tone::Alert),
            (None, Some(q)) if q.reset > now => {
                if q.remaining == 0 {
                    let reset = q.reset.with_timezone(&self.tz).format("%H:%M");
                    segment(format!("✘ API 0/{} · {reset} 恢复", q.limit), Tone::Alert)
                } else {
                    let low = u64::from(q.remaining) * 5 < u64::from(q.limit);
                    let tone = if low { Tone::Warn } else { Tone::Ok };
                    segment(format!("✔ API {}/{}", q.remaining, q.limit), tone)
                }
            }
            _ => segment("API", Tone::Ok),
        };
        vec![api, segment("guest", Tone::Identity)]
    }

    /// Rows inside the border.
    pub(super) fn inner_height(&self) -> usize {
        usize::from(self.size.1.saturating_sub(2))
    }

    /// Columns for the text of the reader and the help page: inside the border, with a column of
    /// space on each side.
    pub(super) fn reader_width(&self) -> usize {
        usize::from(self.size.0.saturating_sub(4))
    }

    /// Topics on a page of the topic list, which takes two rows for each.
    pub(super) fn topics_page(&self) -> usize {
        (self.inner_height() / 2).max(1)
    }

    /// Pushes the topic list of `listing` and asks for its topics.
    pub(super) fn open_listing(&mut self, listing: Listing, title: String) -> Vec<Cmd> {
        let ticket = self.ticket();
        self.stack.push(Screen::Topics(TopicsScreen::new(
            listing.clone(),
            title,
            ticket,
        )));
        vec![Cmd::LoadTopics {
            ticket,
            listing,
            refresh: false,
        }]
    }

    /// Pushes the reader of topic `id` and asks for the topic and its replies.
    pub(super) fn open_topic(&mut self, id: u64, new_replies: u32) -> Vec<Cmd> {
        let ticket = self.ticket();
        self.stack
            .push(Screen::Reader(ReaderScreen::new(id, ticket, new_replies)));
        vec![Cmd::LoadThread {
            ticket,
            id,
            refresh: false,
        }]
    }

    /// Opens topic `id` from a topic list that shows it with `replies` replies: notes how many
    /// of them are new since it was last recorded and pushes its reader. The topic is recorded
    /// once its thread has loaded.
    pub(super) fn open_from_list(&mut self, id: u64, replies: u32) -> Vec<Cmd> {
        let new_replies = match self.read.mark(id, replies) {
            Mark::Updated(n) => n,
            Mark::Unread | Mark::Read => 0,
        };
        self.open_topic(id, new_replies)
    }

    fn key(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Vec<Cmd> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return Vec::new();
        }
        match self.stack.last() {
            Some(Screen::Nodes(_)) => return self.nodes_key(key),
            Some(Screen::Topics(_)) => return self.topics_key(key, now),
            Some(Screen::Reader(_)) => return self.reader_key(key),
            Some(Screen::Help(_)) => self.help_key(key),
            None => {}
        }
        Vec::new()
    }

    /// Puts the cursor of every list back in range after its rows or the page changed.
    fn clamp_lists(&mut self) {
        let (page, topics_page) = (self.inner_height(), self.topics_page());
        let directory = self.directory.as_deref();
        for screen in &mut self.stack {
            match screen {
                Screen::Nodes(nodes) => {
                    let len = nodes.rows(&self.favorites, directory).len();
                    nodes.nav.clamp(len, page);
                }
                Screen::Topics(topics) => {
                    let len = topics.rows().len();
                    topics.nav.clamp(len, topics_page);
                }
                Screen::Reader(_) | Screen::Help(_) => {}
            }
        }
    }
}

/// Asks for the node directory when there is none and none is on its way; `loading` is whether
/// one is.
pub(super) fn ask_directory(directory: Option<&[Node]>, loading: &mut bool) -> Vec<Cmd> {
    if directory.is_some() || *loading {
        return Vec::new();
    }
    *loading = true;
    vec![Cmd::LoadDirectory { refresh: false }]
}

/// A segment of the top border.
pub(super) fn segment(text: impl Into<String>, tone: Tone) -> Segment {
    Segment {
        text: text.into(),
        tone,
    }
}

/// A key hint for the bottom border.
pub(super) fn hint(key: &str, label: &str) -> (String, String) {
    (key.to_string(), label.to_string())
}

/// Draws the screen on top of the stack, or only a notice when the terminal is too small.
pub fn view(app: &App, theme: &Theme, edge: SegmentEdge, now: DateTime<Utc>, frame: &mut Frame) {
    let area = frame.area();
    if !fits(area) {
        draw_too_small(frame, area);
        return;
    }
    match app.stack.last() {
        Some(Screen::Nodes(screen)) => nodes::draw(app, screen, theme, edge, now, frame),
        Some(Screen::Topics(screen)) => topics::draw(app, screen, theme, edge, now, frame),
        Some(Screen::Reader(screen)) => reader::draw(app, screen, theme, edge, now, frame),
        Some(Screen::Help(screen)) => help::draw(app, screen, theme, edge, now, frame),
        None => {}
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Building an app and pressing keys, for the tests of every screen.

    use chrono::TimeZone;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use unicode_width::UnicodeWidthStr;
    use v2ex_rs::MemberRef;

    use super::*;

    pub fn tz() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    pub fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 19, 1, 0, 0).unwrap()
    }

    /// An app on an 80×24 terminal with the preset favorites and nothing read.
    pub fn launch(start: Start) -> (App, Vec<Cmd>) {
        App::new(
            start,
            Favorites::load(None),
            ReadState::load(None),
            (80, 24),
            tz(),
        )
    }

    pub fn press(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
        app.update(Msg::Key(key), now())
    }

    pub fn code(app: &mut App, code: KeyCode) -> Vec<Cmd> {
        press(app, KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Types each character as a key and returns every command they gave.
    pub fn chars(app: &mut App, text: &str) -> Vec<Cmd> {
        text.chars()
            .flat_map(|c| code(app, KeyCode::Char(c)))
            .collect()
    }

    pub fn ctrl_c(app: &mut App) -> Vec<Cmd> {
        press(
            app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        )
    }

    pub fn node(name: &str, title: &str, topics: u32) -> Node {
        Node {
            id: u64::from(topics),
            name: name.into(),
            title: title.into(),
            title_alternative: None,
            topics,
            stars: 0,
            header: None,
            parent_node_name: None,
        }
    }

    /// `n` nodes, `node1` to `nodeN`, each with as many topics as its number.
    pub fn nodes(n: u32) -> Vec<Node> {
        (1..=n)
            .map(|i| node(&format!("node{i}"), &format!("节点{i}"), i))
            .collect()
    }

    /// Feeds the directory in as if `LoadDirectory` had returned it.
    pub fn load_directory(app: &mut App, nodes: Vec<Node>) -> Vec<Cmd> {
        app.update(
            Msg::DirectoryLoaded {
                result: Ok(nodes),
                quota: None,
            },
            now(),
        )
    }

    pub fn nodes_screen(app: &App) -> &NodesScreen {
        match &app.stack[0] {
            Screen::Nodes(screen) => screen,
            _ => panic!("the bottom screen is not the node list"),
        }
    }

    /// A topic created at [`now`], with a made-up URL.
    pub fn topic(id: u64, title: &str, author: &str, replies: u32) -> Topic {
        Topic {
            id,
            title: title.into(),
            url: format!("https://www.v2ex.com/t/{id}"),
            author: MemberRef {
                username: author.into(),
                id: None,
            },
            node: None,
            created: now(),
            last_touched: None,
            replies,
            last_reply_by: None,
            content: None,
            content_html: None,
        }
    }

    /// A reply posted an hour after [`now`].
    pub fn reply(floor: u32, author: &str, html: &str) -> Reply {
        Reply {
            id: 9000 + u64::from(floor),
            floor,
            author: MemberRef {
                username: author.into(),
                id: None,
            },
            content: String::new(),
            content_html: html.into(),
            created: now() + chrono::Duration::hours(1),
        }
    }

    /// The reader on top of the stack.
    pub fn reader_screen(app: &App) -> &ReaderScreen {
        match app.stack.last() {
            Some(Screen::Reader(reader)) => reader,
            _ => panic!("the reader is not on top"),
        }
    }

    /// Feeds `topic` and `replies` in as the result the reader on top is waiting for.
    pub fn load_thread(app: &mut App, topic: Topic, replies: Vec<Reply>) -> Vec<Cmd> {
        let ticket = reader_screen(app)
            .pending
            .expect("the reader is waiting for its topic");
        app.update(
            Msg::ThreadLoaded {
                ticket,
                result: Ok((topic, replies)),
                quota: None,
            },
            now(),
        )
    }

    /// The topic list nearest the top of the stack.
    pub fn topics_screen(app: &App) -> &TopicsScreen {
        app.stack
            .iter()
            .rev()
            .find_map(|screen| match screen {
                Screen::Topics(topics) => Some(topics),
                _ => None,
            })
            .expect("a topic list is on the stack")
    }

    /// Feeds `topics` in as the result the topic list nearest the top is waiting for.
    pub fn load_topics(app: &mut App, topics: Vec<Topic>) -> Vec<Cmd> {
        let ticket = topics_screen(app)
            .pending
            .expect("the topic list is waiting for topics");
        app.update(
            Msg::TopicsLoaded {
                ticket,
                result: Ok(topics),
                quota: None,
            },
            now(),
        )
    }

    /// An app on the hot listing with `topics` loaded.
    pub fn listing(topics: Vec<Topic>) -> App {
        let (mut app, _) = launch(Start::Listing(Listing::Hot));
        load_topics(&mut app, topics);
        app
    }

    pub fn draw(app: &App, w: u16, h: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| view(app, &Theme { color: true }, SegmentEdge::Plain, now(), f))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// The text of one row; the hidden cells behind a wide character add nothing.
    pub fn row(buf: &Buffer, y: u16) -> String {
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

    pub fn all_text(buf: &Buffer) -> String {
        (0..buf.area.height).map(|y| row(buf, y) + "\n").collect()
    }

    /// Every row is as wide as the terminal and ends in the border.
    pub fn assert_framed(buf: &Buffer) {
        let (w, h) = (buf.area.width, buf.area.height);
        assert_eq!(buf[(0, 0)].symbol(), "╭");
        assert_eq!(buf[(w - 1, 0)].symbol(), "╮");
        assert_eq!(buf[(w - 1, h - 1)].symbol(), "╯");
        for y in 1..h - 1 {
            assert_eq!(buf[(w - 1, y)].symbol(), "│", "row {y}");
        }
        for y in 0..h {
            assert_eq!(row(buf, y).width(), usize::from(w), "row {y}");
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use v2ex_rs::Error;

    use super::testing::*;
    use super::*;
    use crate::tui::nodes::NodesView;

    fn quota(limit: u32, remaining: u32) -> RateLimit {
        RateLimit {
            limit,
            remaining,
            // 13:00 at UTC+8.
            reset: Utc.with_ymd_and_hms(2026, 11, 19, 5, 0, 0).unwrap(),
        }
    }

    fn loaded(ticket: u64, result: v2ex_rs::Result<Vec<Topic>>) -> Msg {
        Msg::TopicsLoaded {
            ticket,
            result,
            quota: None,
        }
    }

    #[test]
    fn loaded_topics_are_shown_and_a_stale_ticket_is_dropped() {
        let (mut app, _) = launch(Start::Listing(Listing::Hot));
        let t1 = topics_screen(&app).pending.unwrap();
        code(&mut app, KeyCode::Char('r'));
        let t2 = topics_screen(&app).pending.unwrap();
        assert_ne!(t1, t2);

        let three = (1..=3).map(|i| topic(i, "示例", "alice", 1)).collect();
        assert!(app.update(loaded(t1, Ok(three)), now()).is_empty());
        assert!(topics_screen(&app).topics.is_empty());
        assert_eq!(topics_screen(&app).pending, Some(t2));

        let two = (1..=2).map(|i| topic(i, "示例", "alice", 1)).collect();
        assert!(app.update(loaded(t2, Ok(two)), now()).is_empty());
        assert_eq!(topics_screen(&app).topics.len(), 2);
        assert_eq!(topics_screen(&app).pending, None);
    }

    #[test]
    fn a_result_for_a_screen_already_left_is_dropped() {
        let (mut app, _) = launch(Start::Listing(Listing::Hot));
        let t1 = topics_screen(&app).pending.unwrap();
        code(&mut app, KeyCode::Left);
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
        let cmds = app.update(loaded(t1, Ok(vec![topic(1, "示例", "alice", 1)])), now());
        assert!(cmds.is_empty());
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
    }

    #[test]
    fn a_result_reaches_the_topic_list_under_the_reader() {
        let mut app = listing(vec![topic(1, "示例", "alice", 1)]);
        code(&mut app, KeyCode::Char('r'));
        let ticket = topics_screen(&app).pending.unwrap();
        code(&mut app, KeyCode::Enter);
        assert!(matches!(app.stack.last(), Some(Screen::Reader(_))));
        let two = (1..=2).map(|i| topic(i, "示例", "alice", 1)).collect();
        assert!(app.update(loaded(ticket, Ok(two)), now()).is_empty());
        assert_eq!(topics_screen(&app).topics.len(), 2);
        assert_eq!(topics_screen(&app).pending, None);
        assert!(matches!(app.stack.last(), Some(Screen::Reader(_))));
    }

    #[test]
    fn a_failed_reload_keeps_the_topics_and_reports_the_reason() {
        let mut app = listing((1..=3).map(|i| topic(i, "示例", "alice", 1)).collect());
        code(&mut app, KeyCode::End);
        code(&mut app, KeyCode::Char('r'));
        let ticket = topics_screen(&app).pending.unwrap();
        let cmds = app.update(
            Msg::TopicsLoaded {
                ticket,
                result: Err(Error::RateLimited {
                    reset: Some(quota(600, 0).reset),
                }),
                quota: Some(quota(600, 0)),
            },
            now(),
        );
        assert!(cmds.is_empty());
        assert_eq!(topics_screen(&app).topics.len(), 3);
        assert_eq!(topics_screen(&app).pending, None);
        let error = app.error.clone().unwrap();
        assert!(error.contains("额度") && error.contains("13:00"), "{error}");
        assert_eq!(app.quota, Some(quota(600, 0)));

        // A later success clears the error and pulls the cursor back onto the shorter list.
        code(&mut app, KeyCode::Char('r'));
        load_topics(&mut app, vec![topic(1, "示例", "alice", 1)]);
        assert!(app.error.is_none());
        assert_eq!(topics_screen(&app).topics.len(), 1);
        assert_eq!(topics_screen(&app).nav.cursor, 0);
    }

    #[test]
    fn starts_on_favorites_and_enter_opens_the_listing() {
        let (mut app, cmds) = launch(Start::Home);
        assert!(cmds.is_empty());
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
        assert_eq!(nodes_screen(&app).view, NodesView::Favorites);

        let cmds = code(&mut app, KeyCode::Enter);
        let [Screen::Nodes(_), Screen::Topics(topics)] = app.stack.as_slice() else {
            panic!("expected [Nodes, Topics]");
        };
        assert_eq!(topics.listing, Listing::Hot);
        assert_eq!(topics.title, "最热");
        let ticket = topics.pending.unwrap();
        assert_eq!(
            cmds,
            [Cmd::LoadTopics {
                ticket,
                listing: Listing::Hot,
                refresh: false
            }]
        );
    }

    #[test]
    fn start_listing_and_start_topic_open_on_top_of_home() {
        let (app, cmds) = launch(Start::Listing(Listing::Node("python".into())));
        let [Screen::Nodes(_), Screen::Topics(topics)] = app.stack.as_slice() else {
            panic!("expected [Nodes, Topics]");
        };
        assert_eq!(topics.title, "python");
        assert_eq!(
            cmds,
            [Cmd::LoadTopics {
                ticket: topics.pending.unwrap(),
                listing: Listing::Node("python".into()),
                refresh: false
            }]
        );

        let (app, cmds) = launch(Start::Listing(Listing::Tab("tech".into())));
        let [Screen::Nodes(_), Screen::Topics(topics)] = app.stack.as_slice() else {
            panic!("expected [Nodes, Topics]");
        };
        assert_eq!(topics.title, "技术");
        assert_eq!(cmds.len(), 1);

        let (app, cmds) = launch(Start::Topic(123));
        let [Screen::Nodes(_), Screen::Reader(reader)] = app.stack.as_slice() else {
            panic!("expected [Nodes, Reader]");
        };
        assert_eq!(reader.id, 123);
        assert_eq!(
            cmds,
            [Cmd::LoadThread {
                ticket: reader.pending.unwrap(),
                id: 123,
                refresh: false
            }]
        );
    }

    #[test]
    fn tickets_count_up() {
        let (mut app, _) = launch(Start::Home);
        let first = app.ticket();
        assert_eq!(app.ticket(), first + 1);
        assert_eq!(app.ticket(), first + 2);
    }

    #[test]
    fn ctrl_c_quits_from_any_state_and_question_mark_opens_help() {
        // While typing a filter.
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('/'));
        assert!(nodes_screen(&app).prompt.is_some());
        assert!(ctrl_c(&mut app).is_empty());
        assert!(app.quit);

        // While moving a favorite.
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('M'));
        assert!(nodes_screen(&app).moving);
        ctrl_c(&mut app);
        assert!(app.quit);

        // While typing a node name on the topic list.
        let (mut app, _) = launch(Start::Listing(Listing::Hot));
        code(&mut app, KeyCode::Char('s'));
        assert!(topics_screen(&app).prompt.is_some());
        ctrl_c(&mut app);
        assert!(app.quit);

        // On every other screen.
        for start in [Start::Listing(Listing::Hot), Start::Topic(1)] {
            let (mut app, _) = launch(start);
            ctrl_c(&mut app);
            assert!(app.quit);
        }
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('?'));
        ctrl_c(&mut app);
        assert!(app.quit);

        let (mut app, _) = launch(Start::Home);
        assert!(code(&mut app, KeyCode::Char('?')).is_empty());
        assert!(matches!(
            app.stack.as_slice(),
            [Screen::Nodes(_), Screen::Help(_)]
        ));
        code(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
        assert!(!app.quit);

        for back in [
            KeyCode::Left,
            KeyCode::Char('h'),
            KeyCode::Char('?'),
            KeyCode::Esc,
        ] {
            code(&mut app, KeyCode::Char('?'));
            code(&mut app, KeyCode::Char('j'));
            assert!(matches!(
                app.stack.as_slice(),
                [Screen::Nodes(_), Screen::Help(_)]
            ));
            code(&mut app, back);
            assert!(
                matches!(app.stack.as_slice(), [Screen::Nodes(_)]),
                "{back:?}"
            );
        }
        assert!(!app.quit);
    }

    #[test]
    fn the_topic_list_and_the_reader_open_help_and_go_back() {
        let mut app = listing(Vec::new());
        code(&mut app, KeyCode::Char('?'));
        assert!(matches!(app.stack.last(), Some(Screen::Help(_))));
        code(&mut app, KeyCode::Esc);
        code(&mut app, KeyCode::Char('j'));
        assert_eq!(app.stack.len(), 2);
        code(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
        assert!(!app.quit);

        let (mut app, _) = launch(Start::Topic(7));
        let cmds = app.update(
            Msg::ThreadLoaded {
                ticket: 99,
                result: Err(Error::NotFound),
                quota: None,
            },
            now(),
        );
        assert!(cmds.is_empty());
        for back in [KeyCode::Left, KeyCode::Char('h')] {
            let (mut app, _) = launch(Start::Topic(7));
            code(&mut app, back);
            assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
        }
        code(&mut app, KeyCode::Char('q'));
        assert!(matches!(app.stack.as_slice(), [Screen::Nodes(_)]));
        assert!(!app.quit);
    }

    #[test]
    fn a_failed_save_is_reported() {
        let (mut app, _) = launch(Start::Home);
        let cmds = app.update(Msg::SaveFailed("无法写入 favorites.json".into()), now());
        assert!(cmds.is_empty());
        assert_eq!(app.error.as_deref(), Some("无法写入 favorites.json"));
    }

    #[test]
    fn resize_keeps_the_cursor_visible_and_survives_tiny_sizes() {
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('y'));
        load_directory(&mut app, nodes(100));
        code(&mut app, KeyCode::End);
        assert_eq!(nodes_screen(&app).nav.cursor, 110);

        app.update(Msg::Resize(80, 10), now());
        assert_eq!(app.size, (80, 10));
        let nav = &nodes_screen(&app).nav;
        assert!(nav.offset <= nav.cursor && nav.cursor < nav.offset + 8);

        app.update(Msg::Resize(10, 2), now());
        draw(&app, 10, 2);
        app.update(Msg::Resize(80, 24), now());
        let nav = &nodes_screen(&app).nav;
        assert_eq!(nav.cursor, 110);
        assert!(nav.offset <= nav.cursor && nav.cursor < nav.offset + 22);
        assert_framed(&draw(&app, 80, 24));
    }

    #[test]
    fn right_segments_follow_the_quota_and_the_last_error() {
        let (mut app, _) = launch(Start::Home);
        let segs = app.right_segments(now());
        assert_eq!(segs.len(), 2);
        assert!(segs[0].text.contains("API"));
        assert_eq!(segs[0].tone, Tone::Ok);
        assert_eq!(
            segs[1],
            Segment {
                text: "guest".into(),
                tone: Tone::Identity
            }
        );

        app.quota = Some(quota(600, 571));
        let segs = app.right_segments(now());
        assert_eq!(segs[0].text, "✔ API 571/600");
        assert_eq!(segs[0].tone, Tone::Ok);

        app.quota = Some(quota(600, 120));
        assert_eq!(app.right_segments(now())[0].tone, Tone::Ok);
        app.quota = Some(quota(600, 100));
        assert_eq!(app.right_segments(now())[0].tone, Tone::Warn);

        app.quota = Some(quota(600, 0));
        let segs = app.right_segments(now());
        assert_eq!(segs[0].tone, Tone::Alert);
        assert!(segs[0].text.contains("13:00"), "{}", segs[0].text);

        // Once the recovery time has passed, the old numbers no longer hold.
        let later = quota(600, 0).reset + Duration::minutes(1);
        let segs = app.right_segments(later);
        assert_eq!(segs[0].text, "API");
        assert_eq!(segs[0].tone, Tone::Ok);

        app.quota = Some(quota(600, 571));
        app.error = Some("网络错误：连接超时".into());
        let segs = app.right_segments(now());
        assert_eq!(segs[0].tone, Tone::Alert);
        assert!(segs[0].text.contains("网络错误：连接超时"));
        assert_eq!(segs.last().unwrap().text, "guest");
        assert_eq!(segs.last().unwrap().tone, Tone::Identity);
    }

    #[test]
    fn a_long_error_is_cut_in_the_top_border_not_hidden() {
        let (mut app, _) = launch(Start::Home);
        app.error = Some(format!("网络错误：{}", "e".repeat(295)));
        let buf = draw(&app, 80, 24);
        assert_framed(&buf);
        let top = row(&buf, 0);
        assert!(
            top.contains("✘ 网络错误：eeee") && top.contains("…"),
            "{top}"
        );
        assert!(!top.contains("guest"), "{top}");
    }

    #[test]
    fn the_node_list_fills_the_height_and_keeps_its_border() {
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('y'));
        load_directory(&mut app, nodes(100));
        for (h, inner) in [(24, 22), (12, 10)] {
            app.update(Msg::Resize(80, h), now());
            let buf = draw(&app, 80, h);
            assert_framed(&buf);
            let filled = (1..h - 1)
                .filter(|&y| !row(&buf, y).trim_matches(['│', ' ']).is_empty())
                .count();
            assert_eq!(filled, inner);
            assert!(row(&buf, 1).contains("❯"));
            assert!(row(&buf, 1).contains("hot") && row(&buf, 1).contains("最热"));
            assert!(row(&buf, 0).contains("全部节点"));
            assert!(row(&buf, h - 1).contains("1/111"), "{}", row(&buf, h - 1));
        }

        // The last page ends with the last node, the one with the fewest topics.
        code(&mut app, KeyCode::End);
        let buf = draw(&app, 80, 12);
        assert!(row(&buf, 10).contains("❯") && row(&buf, 10).contains("node1 "));
        assert!(row(&buf, 11).contains("111/111"));
    }

    #[test]
    fn every_screen_keeps_its_border_at_the_smallest_size() {
        let mut apps = vec![launch(Start::Home).0, launch(Start::Topic(1)).0];
        let (mut help, _) = launch(Start::Listing(Listing::Hot));
        code(&mut help, KeyCode::Char('?'));
        apps.push(help);
        let (mut all, _) = launch(Start::Home);
        code(&mut all, KeyCode::Char('y'));
        load_directory(&mut all, nodes(100));
        apps.push(all);
        let mut topics = listing(
            (1..=50)
                .map(|i| topic(i, "示例", "alice", i as u32))
                .collect(),
        );
        code(&mut topics, KeyCode::End);
        apps.push(topics);
        let mut goto = listing(Vec::new());
        load_directory(&mut goto, nodes(100));
        chars(&mut goto, &format!("s{}", "node".repeat(20)));
        apps.push(goto);
        let mut reader = reading(long_topic(1, 97), vec![reply(1, "bob", "第一则")]);
        code(&mut reader, KeyCode::End);
        apps.push(reader);
        for mut app in apps {
            for (w, h) in [(40, 8), (41, 9), (80, 24), (200, 60)] {
                app.update(Msg::Resize(w, h), now());
                assert_framed(&draw(&app, w, h));
            }
        }
    }

    #[test]
    fn wide_and_control_characters_in_node_names_stay_inside_the_frame() {
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('y'));
        load_directory(
            &mut app,
            vec![
                node("wide", &"节".repeat(60), 900),
                node("e\x1b[31mvil", "a\x1b[2Jb", 800),
                node("emoji", &"👨‍👩‍👧".repeat(30), 700),
            ],
        );
        for (w, h) in [(40, 16), (41, 16), (80, 16)] {
            app.update(Msg::Resize(w, h), now());
            let buf = draw(&app, w, h);
            assert_framed(&buf);
            let text = all_text(&buf);
            assert!(!text.contains('\x1b'));
            assert!(text.contains("e[31mvil") && text.contains("a[2Jb"));
        }
    }

    #[test]
    fn the_bottom_border_shows_the_prompt_the_typed_number_and_loading() {
        let (mut app, _) = launch(Start::Home);
        let bottom = |app: &App| row(&draw(app, 80, 24), 23);
        assert!(bottom(&app).contains("y 全部节点"));
        assert!(bottom(&app).contains("1/7"));

        chars(&mut app, "12");
        assert!(bottom(&app).contains("12"));
        code(&mut app, KeyCode::Esc);
        assert!(!bottom(&app).contains("12"));

        chars(&mut app, "/技");
        assert!(bottom(&app).contains("过滤：") && bottom(&app).contains("技"));
        assert!(bottom(&app).contains("3/7"));
        code(&mut app, KeyCode::Enter);
        assert!(bottom(&app).contains("技"));

        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('y'));
        let buf = draw(&app, 80, 24);
        assert!(row(&buf, 0).contains("…"));
        assert!(row(&buf, 23).contains("y 我的最爱"));
        chars(&mut app, "/zzz");
        assert!(row(&draw(&app, 80, 24), 23).contains("0/0"));
    }

    #[test]
    fn below_the_minimum_size_only_the_notice_is_drawn() {
        let (mut app, _) = launch(Start::Home);
        app.update(Msg::Resize(30, 5), now());
        let text = all_text(&draw(&app, 30, 5));
        assert!(text.contains("终端太小"));
        assert!(!text.contains('╭'));
    }

    /// Topic `id` whose body is `n` short lines, `第1行` to `第n行`: with no replies, `n + 3`
    /// lines with the title, the meta line and the blank line.
    fn long_topic(id: u64, n: usize) -> Topic {
        let mut t = topic(id, "示例", "alice", 0);
        let body: Vec<String> = (1..=n).map(|i| format!("第{i}行")).collect();
        t.content_html = Some(body.join("<br>"));
        t
    }

    /// An 80×24 reader opened by topic number, with `topic` and `replies` loaded.
    fn reading(topic: Topic, replies: Vec<Reply>) -> App {
        let (mut app, _) = launch(Start::Topic(topic.id));
        load_thread(&mut app, topic, replies);
        app
    }

    fn scroll(app: &App) -> usize {
        reader_screen(app).scroll
    }

    fn line_count(app: &App) -> usize {
        reader_screen(app).layout.lines.len()
    }

    fn shift(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn thread_loaded(ticket: u64, result: v2ex_rs::Result<(Topic, Vec<Reply>)>) -> Msg {
        Msg::ThreadLoaded {
            ticket,
            result,
            quota: None,
        }
    }

    #[test]
    fn a_loaded_thread_is_laid_out_recorded_and_a_stale_one_is_dropped() {
        let (mut app, _) = launch(Start::Topic(1));
        let t1 = reader_screen(&app).pending.unwrap();
        let cmds = code(&mut app, KeyCode::Char('r'));
        let t2 = reader_screen(&app).pending.unwrap();
        assert_ne!(t1, t2);
        assert_eq!(
            cmds,
            [Cmd::LoadThread {
                ticket: t2,
                id: 1,
                refresh: true
            }]
        );

        let thread = || {
            (
                topic(1, "示例", "alice", 2),
                vec![reply(1, "bob", "第一则"), reply(2, "carol", "第二则")],
            )
        };
        assert!(
            app.update(thread_loaded(t1, Ok(thread())), now())
                .is_empty()
        );
        let reader = reader_screen(&app);
        assert!(reader.layout.lines.is_empty() && reader.topic.is_none());
        assert_eq!(reader.pending, Some(t2));
        assert_eq!(app.read.mark(1, 2), Mark::Unread);

        let cmds = app.update(thread_loaded(t2, Ok(thread())), now());
        assert!(cmds.contains(&Cmd::SaveRead), "{cmds:?}");
        let reader = reader_screen(&app);
        assert!(!reader.layout.lines.is_empty());
        assert_eq!(reader.pending, None);
        assert_eq!(reader.replies.len(), 2);
        assert_eq!(app.read.mark(1, 2), Mark::Read);
        assert_eq!(app.read.mark(1, 3), Mark::Updated(1));
    }

    #[test]
    fn a_topic_whose_first_load_fails_stays_unread() {
        let mut app = listing(vec![
            topic(1, "示例一", "alice", 1),
            topic(2, "示例二", "bob", 2),
        ]);
        let opens = [
            (KeyCode::Enter, Error::Network("连接超时".into())),
            (
                KeyCode::Char(']'),
                Error::Restricted {
                    node: "示例节点".into(),
                },
            ),
        ];
        for (key, error) in opens {
            assert!(!code(&mut app, key).contains(&Cmd::SaveRead), "{key:?}");
            let ticket = reader_screen(&app).pending.unwrap();
            assert!(
                app.update(thread_loaded(ticket, Err(error)), now())
                    .is_empty()
            );
        }
        assert_eq!(app.read.mark(1, 1), Mark::Unread);
        assert_eq!(app.read.mark(2, 2), Mark::Unread);
    }

    #[test]
    fn a_loaded_thread_is_recorded_with_the_larger_reply_count() {
        // The list counts 5 replies; the thread comes with 3 of them.
        let mut app = listing(vec![topic(1, "示例", "alice", 5)]);
        code(&mut app, KeyCode::Enter);
        let replies = (1..=3).map(|i| reply(i, "bob", "第一则")).collect();
        let cmds = load_thread(&mut app, topic(1, "示例", "alice", 5), replies);
        assert_eq!(cmds, [Cmd::SaveRead]);
        assert_eq!(app.read.mark(1, 5), Mark::Read);

        // The thread comes with more replies than its count.
        let (mut app, _) = launch(Start::Topic(1));
        let replies = (1..=3).map(|i| reply(i, "bob", "第一则")).collect();
        load_thread(&mut app, topic(1, "示例", "alice", 1), replies);
        assert_eq!(app.read.mark(1, 3), Mark::Read);
    }

    #[test]
    fn a_failed_load_keeps_what_was_shown() {
        let mut app = reading(long_topic(1, 30), vec![reply(1, "bob", "第一则")]);
        let shown = reader_screen(&app).layout.clone();
        code(&mut app, KeyCode::Char('r'));
        let ticket = reader_screen(&app).pending.unwrap();
        let failed = Err(Error::Network("连接超时".into()));
        assert!(app.update(thread_loaded(ticket, failed), now()).is_empty());
        assert_eq!(reader_screen(&app).layout, shown);
        assert_eq!(reader_screen(&app).pending, None);
        let error = app.error.clone().unwrap();
        assert!(error.contains("连接超时"), "{error}");

        // A first load that fails leaves an empty reader where every key is safe.
        let (mut app, _) = launch(Start::Topic(1));
        let ticket = reader_screen(&app).pending.unwrap();
        let restricted = Err(Error::Restricted {
            node: "示例节点".into(),
        });
        app.update(thread_loaded(ticket, restricted), now());
        assert!(reader_screen(&app).layout.lines.is_empty());
        let error = app.error.clone().unwrap();
        assert!(error.contains("需要登录"), "{error}");
        let keys = [
            plain(KeyCode::Down),
            plain(KeyCode::Up),
            plain(KeyCode::PageDown),
            plain(KeyCode::PageUp),
            plain(KeyCode::End),
            plain(KeyCode::Home),
            plain(KeyCode::Enter),
            plain(KeyCode::Esc),
            ctrl('f'),
            ctrl('b'),
            shift('G'),
            shift('N'),
        ];
        for key in keys {
            assert!(press(&mut app, key).is_empty(), "{key:?}");
        }
        assert!(chars(&mut app, "jkgGnN ][o2").is_empty());
        assert!(code(&mut app, KeyCode::Enter).is_empty());
        assert!(chars(&mut app, "/示例").is_empty());
        assert!(code(&mut app, KeyCode::Enter).is_empty());
        assert!(chars(&mut app, "n").is_empty());
        assert_eq!(scroll(&app), 0);
        assert_eq!(app.stack.len(), 2);
        let buf = draw(&app, 80, 24);
        assert_framed(&buf);
        assert!(row(&buf, 23).contains("100%"), "{}", row(&buf, 23));
    }

    #[test]
    fn scrolling_and_paging_stay_in_range() {
        let mut app = reading(long_topic(1, 97), Vec::new());
        assert_eq!(line_count(&app), 100);
        assert_eq!(app.inner_height(), 22);
        assert!(row(&draw(&app, 80, 24), 23).contains("22%"));
        let steps = [
            (plain(KeyCode::Char('j')), 1),
            (plain(KeyCode::Char(' ')), 23),
            (plain(KeyCode::Down), 24),
            (plain(KeyCode::Char('k')), 23),
            (plain(KeyCode::Up), 22),
            (plain(KeyCode::PageDown), 44),
            (ctrl('f'), 66),
            (plain(KeyCode::PageUp), 44),
            (ctrl('b'), 22),
            (plain(KeyCode::End), 78),
            (plain(KeyCode::Char('j')), 78),
            (plain(KeyCode::PageDown), 78),
            (plain(KeyCode::Home), 0),
            (plain(KeyCode::Char('k')), 0),
            (plain(KeyCode::PageUp), 0),
            (shift('G'), 78),
            (plain(KeyCode::Char('g')), 0),
            (plain(KeyCode::Char('G')), 78),
        ];
        for (key, expected) in steps {
            press(&mut app, key);
            assert_eq!(scroll(&app), expected, "{key:?}");
        }
        assert!(row(&draw(&app, 80, 24), 23).contains("100%"));

        // Shorter than a page: nothing scrolls and the position is the whole.
        let mut app = reading(long_topic(1, 2), Vec::new());
        assert_eq!(line_count(&app), 5);
        for key in [
            plain(KeyCode::Char('j')),
            plain(KeyCode::Down),
            plain(KeyCode::Char(' ')),
            plain(KeyCode::PageDown),
            ctrl('f'),
            plain(KeyCode::End),
            shift('G'),
            plain(KeyCode::PageUp),
            plain(KeyCode::Home),
        ] {
            press(&mut app, key);
            assert_eq!(scroll(&app), 0, "{key:?}");
        }
        assert!(row(&draw(&app, 80, 24), 23).contains("100%"));
    }

    #[test]
    fn a_number_then_enter_jumps_to_that_floor() {
        // 13 lines of topic, a blank line, then 30 blocks of 2 lines: 74 lines, at most 52.
        let replies = (1..=30)
            .map(|i| reply(i, "bob", &format!("第{i}则")))
            .collect();
        let mut app = reading(long_topic(1, 10), replies);
        let floors = reader_screen(&app).layout.floors.clone();
        assert_eq!(floors.len(), 30);
        let max = line_count(&app) - 22;
        assert_eq!(max, 52);

        assert!(chars(&mut app, "2").is_empty());
        assert_eq!(reader_screen(&app).typed, "2");
        assert!(code(&mut app, KeyCode::Enter).is_empty());
        assert_eq!(scroll(&app), floors[1].min(max));
        assert_eq!(scroll(&app), 16);
        assert_eq!(reader_screen(&app).typed, "");

        chars(&mut app, "30");
        code(&mut app, KeyCode::Enter);
        assert_eq!(scroll(&app), floors[29].min(max));
        chars(&mut app, "1");
        code(&mut app, KeyCode::Enter);
        assert_eq!(scroll(&app), floors[0]);

        // No such floor.
        for number in ["99", "0", "99999999999999999999999"] {
            chars(&mut app, number);
            code(&mut app, KeyCode::Enter);
            assert_eq!(scroll(&app), floors[0], "{number}");
        }

        // Esc clears the digits; any other key clears them and does its own work.
        code(&mut app, KeyCode::Home);
        chars(&mut app, "3");
        code(&mut app, KeyCode::Esc);
        assert_eq!(reader_screen(&app).typed, "");
        code(&mut app, KeyCode::Enter);
        assert_eq!(scroll(&app), 0);
        chars(&mut app, "3j");
        assert_eq!(reader_screen(&app).typed, "");
        assert_eq!(scroll(&app), 1);
        code(&mut app, KeyCode::Enter);
        assert_eq!(scroll(&app), 1);
    }

    #[test]
    fn bracket_keys_walk_the_list_and_space_at_the_end_goes_on() {
        let mut app = listing((1..=3).map(|i| topic(i, "示例", "alice", 1)).collect());
        code(&mut app, KeyCode::Enter);
        let first = reader_screen(&app).pending.unwrap();
        assert_eq!(reader_screen(&app).id, 1);

        let cmds = code(&mut app, KeyCode::Char(']'));
        let [
            Screen::Nodes(_),
            Screen::Topics(list),
            Screen::Reader(reader),
        ] = app.stack.as_slice()
        else {
            panic!("expected [Nodes, Topics, Reader]");
        };
        assert_eq!(reader.id, 2);
        assert_eq!(list.nav.cursor, 1);
        let ticket = reader.pending.unwrap();
        assert_eq!(
            cmds,
            [Cmd::LoadThread {
                ticket,
                id: 2,
                refresh: false
            }]
        );
        // The result for the reader that was replaced is dropped.
        let late = Ok((topic(1, "示例", "alice", 1), Vec::new()));
        assert!(app.update(thread_loaded(first, late), now()).is_empty());
        assert_eq!(reader_screen(&app).pending, Some(ticket));

        code(&mut app, KeyCode::Char('['));
        assert_eq!(reader_screen(&app).id, 1);
        assert_eq!(topics_screen(&app).nav.cursor, 0);
        assert!(code(&mut app, KeyCode::Char('[')).is_empty());
        assert_eq!(reader_screen(&app).id, 1);
        chars(&mut app, "]]");
        assert_eq!(reader_screen(&app).id, 3);
        assert_eq!(topics_screen(&app).nav.cursor, 2);
        assert!(code(&mut app, KeyCode::Char(']')).is_empty());
        assert_eq!(reader_screen(&app).id, 3);
        assert_eq!(app.stack.len(), 3);

        // Space pages until the end, then goes on like ].
        chars(&mut app, "[");
        load_thread(&mut app, long_topic(2, 60), Vec::new());
        code(&mut app, KeyCode::Char(' '));
        assert_eq!((reader_screen(&app).id, scroll(&app)), (2, 22));
        code(&mut app, KeyCode::End);
        let cmds = code(&mut app, KeyCode::Char(' '));
        assert_eq!(reader_screen(&app).id, 3);
        assert_eq!(topics_screen(&app).nav.cursor, 2);
        assert!(matches!(cmds[0], Cmd::LoadThread { id: 3, .. }), "{cmds:?}");

        // Opened by number, there is no list to walk.
        let (mut app, _) = launch(Start::Topic(5));
        for key in [']', '[', ' '] {
            assert!(code(&mut app, KeyCode::Char(key)).is_empty(), "{key}");
            assert_eq!(reader_screen(&app).id, 5);
            assert_eq!(app.stack.len(), 2);
        }
    }

    #[test]
    fn bracket_keys_go_on_from_the_cursor_when_the_open_topic_left_the_list() {
        // On the list of topics 1 to 4, the topic on row `row` is opened while a reload is on
        // its way; the reload comes back as `reloaded`, without it.
        let opened_then_reloaded = |row: usize, reloaded: &[u64]| {
            let mut app = listing((1..=4).map(|i| topic(i, "示例", "alice", 1)).collect());
            for _ in 0..row {
                code(&mut app, KeyCode::Char('j'));
            }
            code(&mut app, KeyCode::Char('r'));
            code(&mut app, KeyCode::Enter);
            assert_eq!(reader_screen(&app).id, row as u64 + 1);
            let topics = reloaded.iter().map(|&i| topic(i, "示例", "alice", 1));
            load_topics(&mut app, topics.collect());
            app
        };
        let opened = |app: &App| (reader_screen(app).id, topics_screen(app).nav.cursor);

        // Topic 2 is gone and the cursor stays on row 1, now topic 3.
        let mut app = opened_then_reloaded(1, &[1, 3, 4]);
        code(&mut app, KeyCode::Char(']'));
        assert_eq!(opened(&app), (3, 1));
        let mut app = opened_then_reloaded(1, &[1, 3, 4]);
        code(&mut app, KeyCode::Char('['));
        assert_eq!(opened(&app), (1, 0));

        // Topic 1 is gone and nothing is before the cursor's row.
        let mut app = opened_then_reloaded(0, &[2, 3, 4]);
        assert!(code(&mut app, KeyCode::Char('[')).is_empty());
        assert_eq!(reader_screen(&app).id, 1);
        code(&mut app, KeyCode::Char(']'));
        assert_eq!(opened(&app), (2, 0));
    }

    #[test]
    fn space_goes_on_only_from_a_loaded_thread() {
        let mut app = listing((1..=3).map(|i| topic(i, "示例", "alice", 1)).collect());
        code(&mut app, KeyCode::Enter);
        // Still loading: space does nothing.
        for _ in 0..2 {
            assert!(code(&mut app, KeyCode::Char(' ')).is_empty());
            assert_eq!(reader_screen(&app).id, 1);
            assert!(reader_screen(&app).pending.is_some());
        }
        assert_eq!(topics_screen(&app).nav.cursor, 0);

        // Reloading a shown thread is loading too.
        load_thread(&mut app, long_topic(1, 2), Vec::new());
        code(&mut app, KeyCode::Char('r'));
        assert!(code(&mut app, KeyCode::Char(' ')).is_empty());
        assert_eq!(reader_screen(&app).id, 1);

        load_thread(&mut app, long_topic(1, 2), Vec::new());
        let cmds = code(&mut app, KeyCode::Char(' '));
        assert_eq!(reader_screen(&app).id, 2);
        assert!(matches!(cmds[0], Cmd::LoadThread { id: 2, .. }), "{cmds:?}");

        // A first load that failed shows nothing to go on from; ] still goes on.
        let ticket = reader_screen(&app).pending.unwrap();
        let failed = Err(Error::Network("连接超时".into()));
        app.update(thread_loaded(ticket, failed), now());
        assert!(code(&mut app, KeyCode::Char(' ')).is_empty());
        assert_eq!(reader_screen(&app).id, 2);
        code(&mut app, KeyCode::Char(']'));
        assert_eq!(reader_screen(&app).id, 3);
    }

    #[test]
    fn search_moves_between_matches_without_wrapping() {
        // Body lines 11, 31 and 51 are laid out as lines 13, 33 and 53.
        let mut t = topic(1, "示例", "alice", 0);
        let body: Vec<String> = (1..=97)
            .map(|i| match i {
                11 => "用 uv 管理依赖".to_string(),
                31 => "UV 的锁文件".to_string(),
                51 => "从 pip 换到 uv".to_string(),
                _ => format!("第{i}行"),
            })
            .collect();
        t.content_html = Some(body.join("<br>"));
        let mut app = reading(t, Vec::new());

        // Without a search, n and N do nothing.
        assert!(chars(&mut app, "nN").is_empty());
        assert_eq!(scroll(&app), 0);

        assert!(chars(&mut app, "/uv").is_empty());
        assert!(reader_screen(&app).prompt.is_some());
        code(&mut app, KeyCode::Enter);
        assert!(reader_screen(&app).prompt.is_none());
        assert_eq!(reader_screen(&app).search.as_deref(), Some("uv"));
        assert_eq!(scroll(&app), 13);
        for (key, expected) in [
            (plain(KeyCode::Char('n')), 33),
            (plain(KeyCode::Char('n')), 53),
            (plain(KeyCode::Char('n')), 53),
            (shift('N'), 33),
            (shift('N'), 13),
            (shift('N'), 13),
            (plain(KeyCode::Char('n')), 33),
        ] {
            press(&mut app, key);
            assert_eq!(scroll(&app), expected, "{key:?}");
        }

        // Esc and ← close the prompt and keep the search; keys are text in the prompt.
        chars(&mut app, "/qhn");
        assert_eq!(
            reader_screen(&app).prompt.as_ref().map(|p| p.text.as_str()),
            Some("qhn")
        );
        assert_eq!(app.stack.len(), 2);
        code(&mut app, KeyCode::Esc);
        assert!(reader_screen(&app).prompt.is_none());
        chars(&mut app, "/zz");
        code(&mut app, KeyCode::Left);
        assert!(reader_screen(&app).prompt.is_none());
        assert_eq!(app.stack.len(), 2);
        assert_eq!(reader_screen(&app).search.as_deref(), Some("uv"));
        assert_eq!(scroll(&app), 33);
        chars(&mut app, "n");
        assert_eq!(scroll(&app), 53);

        // A word that is nowhere moves nothing.
        chars(&mut app, "/不存在");
        code(&mut app, KeyCode::Enter);
        assert_eq!(scroll(&app), 53);
        chars(&mut app, "nN");
        assert_eq!(scroll(&app), 53);
    }

    fn current(app: &App) -> Option<usize> {
        reader_screen(app).current
    }

    #[test]
    fn search_steps_through_every_match_where_the_scroll_stops_short() {
        // Body lines 28, 83 and 93 are laid out as lines 30, 85 and 95 of 100; a 22-row page
        // stops scrolling at line 78.
        let mut t = topic(1, "示例", "alice", 0);
        let body: Vec<String> = (1..=97)
            .map(|i| match i {
                28 | 83 | 93 => format!("用 uv 的第{i}行"),
                _ => format!("第{i}行"),
            })
            .collect();
        t.content_html = Some(body.join("<br>"));
        let mut app = reading(t, Vec::new());
        assert_eq!((line_count(&app), app.inner_height()), (100, 22));

        chars(&mut app, "/uv");
        code(&mut app, KeyCode::Enter);
        assert_eq!((current(&app), scroll(&app)), (Some(30), 30));
        for (key, expected) in [
            (plain(KeyCode::Char('n')), (Some(85), 78)),
            (plain(KeyCode::Char('n')), (Some(95), 78)),
            (plain(KeyCode::Char('n')), (Some(95), 78)),
            (shift('N'), (Some(85), 78)),
            (shift('N'), (Some(30), 30)),
        ] {
            press(&mut app, key);
            assert_eq!((current(&app), scroll(&app)), expected, "{key:?}");
        }

        // Once the current match is off the page, n and N go on from the page's top line.
        chars(&mut app, "nn");
        assert_eq!(current(&app), Some(95));
        code(&mut app, KeyCode::Home);
        chars(&mut app, "n");
        assert_eq!((current(&app), scroll(&app)), (Some(30), 30));

        // A resize that keeps the layout keeps the current match; one that changes it drops it.
        app.update(Msg::Resize(80, 30), now());
        assert_eq!(current(&app), Some(30));
        app.update(Msg::Resize(20, 24), now());
        assert_eq!(current(&app), None);
        app.update(Msg::Resize(80, 24), now());
        chars(&mut app, "n");
        assert_eq!(current(&app), Some(85));

        // A new search starts from the top line of the page.
        code(&mut app, KeyCode::Home);
        chars(&mut app, "/第93");
        code(&mut app, KeyCode::Enter);
        assert_eq!(current(&app), Some(95));
    }

    #[test]
    fn resize_reflows_and_keeps_scroll_in_range() {
        // Each body line is 90 columns: two lines at 76 columns, one at 100.
        let mut t = topic(1, "示例", "alice", 0);
        t.content_html = Some(vec!["字".repeat(45); 60].join("<br>"));
        let mut app = reading(t, Vec::new());
        let before = line_count(&app);
        assert_eq!(before, 123);
        code(&mut app, KeyCode::End);
        assert_eq!(scroll(&app), before - 22);

        app.update(Msg::Resize(160, 50), now());
        assert_eq!(line_count(&app), 63);
        assert_eq!(scroll(&app), 63 - 48);

        app.update(Msg::Resize(20, 4), now());
        draw(&app, 20, 4);
        assert!(scroll(&app) <= line_count(&app).saturating_sub(2));
        app.update(Msg::Resize(80, 24), now());
        assert_eq!(line_count(&app), before);
        assert!(scroll(&app) <= before - 22);
        assert_framed(&draw(&app, 80, 24));
    }

    #[test]
    fn opening_a_topic_with_more_replies_than_recorded_reports_the_difference() {
        let mut app = listing(vec![
            topic(1, "示例一", "alice", 8),
            topic(2, "示例二", "bob", 4),
            topic(3, "示例三", "carol", 2),
        ]);
        app.read.record(1, 5, now());
        app.read.record(2, 1, now());
        code(&mut app, KeyCode::Enter);
        assert_eq!(reader_screen(&app).new_replies, 3);

        // ] and [ count the same way; an unread topic, or one read since, has none new.
        code(&mut app, KeyCode::Char(']'));
        assert_eq!(reader_screen(&app).new_replies, 3);
        load_thread(&mut app, topic(2, "示例二", "bob", 4), Vec::new());
        code(&mut app, KeyCode::Char(']'));
        assert_eq!(reader_screen(&app).new_replies, 0);
        code(&mut app, KeyCode::Char('['));
        assert_eq!(reader_screen(&app).new_replies, 0);

        let (app, _) = launch(Start::Topic(1));
        assert_eq!(reader_screen(&app).new_replies, 0);
    }

    #[test]
    fn the_reader_keeps_its_border_and_shows_the_position() {
        let mut app = listing(vec![topic(1, "示例", "alice", 8)]);
        app.read.record(1, 6, now());
        code(&mut app, KeyCode::Enter);
        let top = row(&draw(&app, 80, 24), 0);
        assert!(top.contains("…") && !top.contains("新回复"), "{top}");

        let mut t = long_topic(1, 40);
        t.replies = 8;
        t.node = Some(v2ex_rs::NodeRef {
            name: Some("python".into()),
            title: "Python".into(),
        });
        let replies = (1..=8).map(|i| reply(i, "bob", "第一则")).collect();
        load_thread(&mut app, t, replies);
        for (w, h) in [(80, 24), (40, 8)] {
            app.update(Msg::Resize(w, h), now());
            let buf = draw(&app, w, h);
            assert_framed(&buf);
            assert!(row(&buf, h - 1).contains('%'), "{}", row(&buf, h - 1));
        }

        app.update(Msg::Resize(80, 24), now());
        let buf = draw(&app, 80, 24);
        let (top, bottom) = (row(&buf, 0), row(&buf, 23));
        assert!(
            top.contains("Python") && top.contains("~ 2 则新回复") && !top.contains("…"),
            "{top}"
        );
        for hint in ["↑↓ 滚动", "空格 翻页", "] 下一篇", "← 返回"] {
            assert!(bottom.contains(hint), "{hint}: {bottom}");
        }
        assert!(row(&buf, 1).contains("示例"));
        assert!(row(&buf, 2).contains("alice · 2026-11-19 09:00 · 8 回复"));

        chars(&mut app, "12");
        assert!(row(&draw(&app, 80, 24), 23).contains("12"));
        code(&mut app, KeyCode::Esc);
        chars(&mut app, "/第一");
        let bottom = row(&draw(&app, 80, 24), 23);
        assert!(
            bottom.contains("查找：") && bottom.contains("第一"),
            "{bottom}"
        );
    }
}
