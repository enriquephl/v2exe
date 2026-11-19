//! The interface as a state machine: keys and request results go in as [`Msg`], requests and
//! saves come out as [`Cmd`], and [`view`] draws one frame from the state. Nothing here touches
//! the terminal, the network, the files or the clock.

use chrono::{DateTime, FixedOffset, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::widgets::Paragraph;
use v2ex_rs::{Listing, Node, RateLimit, Reply, Topic};

use super::Start;
use super::frame::{Chrome, Segment, Tone, draw_frame, draw_too_small, fits};
use super::nodes::{self, NodesScreen, listing_name};
use super::reader::ReaderScreen;
use super::theme::{Paint, Theme};
use super::topics::{self, TopicsScreen};
use crate::errors::message;
use crate::store::config::SegmentEdge;
use crate::store::favorites::{Favorites, listing_title};
use crate::store::read_state::ReadState;

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

pub enum Screen {
    Nodes(NodesScreen),
    Topics(TopicsScreen),
    Reader(ReaderScreen),
    Help,
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
            Start::Topic(id) => app.open_topic(id),
        };
        (app, cmds)
    }

    pub fn update(&mut self, msg: Msg, now: DateTime<Utc>) -> Vec<Cmd> {
        match msg {
            Msg::Key(key) => self.key(key, now),
            Msg::Resize(columns, rows) => {
                self.size = (columns, rows);
                self.clamp_lists();
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
            Msg::ThreadLoaded { .. } => Vec::new(),
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
    pub(super) fn open_topic(&mut self, id: u64) -> Vec<Cmd> {
        let ticket = self.ticket();
        self.stack
            .push(Screen::Reader(ReaderScreen::new(id, ticket)));
        vec![Cmd::LoadThread {
            ticket,
            id,
            refresh: false,
        }]
    }

    fn key(&mut self, key: KeyEvent, now: DateTime<Utc>) -> Vec<Cmd> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return Vec::new();
        }
        match self.stack.last() {
            Some(Screen::Nodes(_)) => return self.nodes_key(key),
            Some(Screen::Topics(_)) => return self.topics_key(key, now),
            Some(Screen::Reader(_)) => match key.code {
                KeyCode::Left | KeyCode::Char('q' | 'h') => {
                    self.stack.pop();
                }
                KeyCode::Char('?') => self.stack.push(Screen::Help),
                _ => {}
            },
            Some(Screen::Help) => {
                if let KeyCode::Left | KeyCode::Esc | KeyCode::Char('q' | 'h' | '?') = key.code {
                    self.stack.pop();
                }
            }
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
                Screen::Reader(_) | Screen::Help => {}
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
    let chrome = |left, hints| Chrome {
        left,
        right: app.right_segments(now),
        hints,
        position: String::new(),
    };
    match app.stack.last() {
        Some(Screen::Nodes(screen)) => nodes::draw(app, screen, theme, edge, now, frame),
        Some(Screen::Topics(screen)) => topics::draw(app, screen, theme, edge, now, frame),
        Some(Screen::Reader(screen)) => {
            let mut left = vec![segment("V2EX", Tone::Site)];
            if screen.pending.is_some() {
                left.push(segment("…", Tone::Status));
            }
            let hints = vec![hint("←", "返回"), hint("?", "说明")];
            draw_frame(frame, area, &chrome(left, hints), theme, edge);
        }
        Some(Screen::Help) => {
            let left = vec![segment("V2EX", Tone::Site), segment("说明", Tone::Location)];
            let hints = vec![hint("←", "返回")];
            let inner = draw_frame(frame, area, &chrome(left, hints), theme, edge);
            let body = Paragraph::new("← 返回   Ctrl-C 退出").style(theme.style(Paint::Plain));
            frame.render_widget(body, inner);
        }
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
            [Screen::Nodes(_), Screen::Help]
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
                [Screen::Nodes(_), Screen::Help]
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
        assert!(matches!(app.stack.last(), Some(Screen::Help)));
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
}
