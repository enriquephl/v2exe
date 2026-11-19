//! The start screen: the favorites, or every feed, tab and node.

use std::cmp::Reverse;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use v2ex_rs::{Listing, Node};

use super::app::{App, Cmd, Screen, ask_directory, hint, segment};
use super::frame::{Chrome, Tone, draw_frame};
use super::list::{ListNav, Nav};
use super::prompt::{Prompt, PromptKey};
use super::theme::{Paint, Theme};
use super::topics::{GOTO, goto_hints};
use crate::render::sanitize;
use crate::store::config::SegmentEdge;
use crate::store::favorites::{Favorite, Favorites, TABS, listing_title};

/// Which list the screen shows.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum NodesView {
    #[default]
    Favorites,
    /// The hot and latest feeds, the tabs, then the node directory.
    All,
}

#[derive(Debug, Default)]
pub struct NodesScreen {
    pub view: NodesView,
    pub nav: ListNav,
    /// Rows are kept when their name or title contains this, ignoring case.
    pub filter: Option<String>,
    /// The filter or the goto input being typed.
    pub prompt: Option<Prompt>,
    /// The favorite under the cursor moves with the up and down keys.
    pub moving: bool,
}

/// One line of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The position in the unfiltered list, counting from 1.
    pub number: usize,
    pub listing: Listing,
    /// `hot`, `latest`, the tab name or the node name.
    pub name: String,
    /// The display name.
    pub title: String,
    /// The node's topic count; `None` for the feeds, the tabs and the favorites.
    pub topics: Option<u32>,
}

impl NodesScreen {
    /// The current view's rows after the filter. The all-nodes view lists the hot and latest
    /// feeds and the nine tabs first, then the node directory by topic count, most first.
    pub fn rows(&self, favorites: &Favorites, directory: Option<&[Node]>) -> Vec<Row> {
        let mut rows = self.all_rows(favorites, directory);
        if let Some(filter) = &self.filter {
            let filter = filter.to_lowercase();
            rows.retain(|row| {
                row.name.to_lowercase().contains(&filter)
                    || row.title.to_lowercase().contains(&filter)
            });
        }
        rows
    }

    /// The current view's rows before the filter.
    fn all_rows(&self, favorites: &Favorites, directory: Option<&[Node]>) -> Vec<Row> {
        let entries: Vec<(Listing, String, Option<u32>)> = match self.view {
            NodesView::Favorites => favorites
                .items()
                .iter()
                .map(|favorite| (favorite.listing.clone(), favorite.title.clone(), None))
                .collect(),
            NodesView::All => {
                let feeds = [Listing::Hot, Listing::Latest]
                    .into_iter()
                    .chain(TABS.iter().map(|(name, _)| Listing::Tab(name.to_string())))
                    .map(|listing| {
                        let title =
                            listing_title(&listing).expect("every feed and tab has a title");
                        (listing, title.to_string(), None)
                    });
                let mut nodes: Vec<&Node> = directory.unwrap_or_default().iter().collect();
                nodes.sort_by_key(|node| Reverse(node.topics));
                let nodes = nodes.into_iter().map(|node| {
                    let listing = Listing::Node(node.name.clone());
                    (listing, node.title.clone(), Some(node.topics))
                });
                feeds.chain(nodes).collect()
            }
        };
        entries
            .into_iter()
            .enumerate()
            .map(|(i, (listing, title, topics))| Row {
                number: i + 1,
                name: listing_name(&listing).to_string(),
                listing,
                title,
                topics,
            })
            .collect()
    }

    /// Sets the filter (an empty one is none) and keeps the cursor on the same row when the row
    /// is still shown, else puts it on the first row.
    fn set_filter(
        &mut self,
        filter: Option<String>,
        favorites: &Favorites,
        directory: Option<&[Node]>,
        page: usize,
    ) {
        let current = self
            .rows(favorites, directory)
            .get(self.nav.cursor)
            .map(|row| row.number);
        self.filter = filter.filter(|filter| !filter.is_empty());
        let rows = self.rows(favorites, directory);
        self.nav.cursor = current
            .and_then(|number| rows.iter().position(|row| row.number == number))
            .unwrap_or(0);
        self.nav.clamp(rows.len(), page);
    }

    /// A key while moving a favorite: up and down swap it with its neighbour. Moving starts
    /// only without a filter, so the cursor is the favorite's index.
    fn move_key(&mut self, key: KeyEvent, favorites: &mut Favorites, page: usize) -> Vec<Cmd> {
        let from = self.nav.cursor;
        let to = match key.code {
            KeyCode::Up | KeyCode::Char('k') => from.checked_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                Some(from + 1).filter(|&to| to < favorites.items().len())
            }
            KeyCode::Char('M') | KeyCode::Enter | KeyCode::Esc => {
                self.moving = false;
                None
            }
            _ => None,
        };
        let Some(to) = to else {
            return Vec::new();
        };
        favorites.swap(from, to);
        self.nav.cursor = to;
        self.nav.clamp(favorites.items().len(), page);
        vec![Cmd::SaveFavorites]
    }

    /// The keys of the node list that are not list keys: `y`, `a`, `d`, `M`, `/`, `s` and `r`.
    /// `loading` is whether the node directory is on its way.
    fn command(
        &mut self,
        key: KeyEvent,
        favorites: &mut Favorites,
        directory: Option<&[Node]>,
        loading: &mut bool,
        page: usize,
    ) -> Vec<Cmd> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Vec::new();
        }
        let rows = self.rows(favorites, directory);
        let row = rows.get(self.nav.cursor);
        match (key.code, self.view) {
            (KeyCode::Char('y'), view) => {
                let view = match view {
                    NodesView::Favorites => NodesView::All,
                    NodesView::All => NodesView::Favorites,
                };
                *self = NodesScreen {
                    view,
                    ..NodesScreen::default()
                };
                match view {
                    NodesView::All => ask_directory(directory, loading),
                    NodesView::Favorites => Vec::new(),
                }
            }
            (KeyCode::Char('a'), NodesView::All) => {
                let Some(row) = row else {
                    return Vec::new();
                };
                let added = favorites.add(Favorite {
                    listing: row.listing.clone(),
                    title: row.title.clone(),
                });
                if added {
                    vec![Cmd::SaveFavorites]
                } else {
                    Vec::new()
                }
            }
            (KeyCode::Char('d'), NodesView::Favorites) => {
                let Some(row) = row else {
                    return Vec::new();
                };
                favorites.remove(row.number - 1);
                let len = self.rows(favorites, directory).len();
                self.nav.clamp(len, page);
                vec![Cmd::SaveFavorites]
            }
            (KeyCode::Char('M'), NodesView::Favorites) => {
                self.moving = row.is_some() && self.filter.is_none();
                Vec::new()
            }
            (KeyCode::Char('/'), _) => {
                self.prompt = Some(Prompt {
                    label: "过滤",
                    text: self.filter.clone().unwrap_or_default(),
                });
                Vec::new()
            }
            (KeyCode::Char('s'), _) => {
                self.prompt = Some(Prompt {
                    label: GOTO,
                    text: String::new(),
                });
                ask_directory(directory, loading)
            }
            (KeyCode::Char('r'), NodesView::All) if !*loading => {
                *loading = true;
                vec![Cmd::LoadDirectory { refresh: true }]
            }
            _ => Vec::new(),
        }
    }
}

/// `hot`, `latest`, or the tab or node name.
pub(super) fn listing_name(listing: &Listing) -> &str {
    match listing {
        Listing::Hot => "hot",
        Listing::Latest => "latest",
        Listing::Tab(name) | Listing::Node(name) => name,
    }
}

impl App {
    /// A key while the node list is on top.
    pub(super) fn nodes_key(&mut self, key: KeyEvent) -> Vec<Cmd> {
        let page = self.inner_height();
        let directory = self.directory.as_deref();
        let Some(Screen::Nodes(screen)) = self.stack.last_mut() else {
            return Vec::new();
        };
        if screen
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.label == GOTO)
        {
            return self.goto_key(key);
        }
        if let Some(prompt) = &mut screen.prompt {
            match prompt.key(key) {
                PromptKey::Edited => {
                    let text = prompt.text.clone();
                    screen.set_filter(Some(text), &self.favorites, directory, page);
                }
                PromptKey::Submit => screen.prompt = None,
                PromptKey::Cancel => {
                    screen.prompt = None;
                    screen.set_filter(None, &self.favorites, directory, page);
                }
                PromptKey::Complete => {}
            }
            return Vec::new();
        }
        if screen.moving {
            return screen.move_key(key, &mut self.favorites, page);
        }
        let rows = screen.rows(&self.favorites, directory);
        match screen.nav.key(key, rows.len(), page) {
            Nav::Handled => Vec::new(),
            Nav::Jump(number) => {
                if let Some(i) = rows.iter().position(|row| row.number == number) {
                    screen.nav.cursor = i;
                    screen.nav.clamp(rows.len(), page);
                }
                Vec::new()
            }
            Nav::Open => match rows.get(screen.nav.cursor) {
                Some(row) => self.open_listing(row.listing.clone(), row.title.clone()),
                None => Vec::new(),
            },
            Nav::Back => {
                if screen.filter.is_some() {
                    screen.set_filter(None, &self.favorites, directory, page);
                } else {
                    self.quit = true;
                }
                Vec::new()
            }
            Nav::Pass if key.code == KeyCode::Char('?') => {
                self.stack.push(Screen::Help);
                Vec::new()
            }
            Nav::Pass => screen.command(
                key,
                &mut self.favorites,
                directory,
                &mut self.directory_loading,
                page,
            ),
        }
    }
}

/// Draws the node list.
pub(super) fn draw(
    app: &App,
    screen: &NodesScreen,
    theme: &Theme,
    edge: SegmentEdge,
    now: DateTime<Utc>,
    frame: &mut Frame,
) {
    let directory = app.directory.as_deref();
    let total = screen.all_rows(&app.favorites, directory).len();
    let rows = screen.rows(&app.favorites, directory);
    let view = match screen.view {
        NodesView::Favorites => "我的最爱",
        NodesView::All => "全部节点",
    };
    let mut left = vec![segment("V2EX", Tone::Site), segment(view, Tone::Location)];
    if app.directory_loading {
        left.push(segment("…", Tone::Status));
    }
    let position = match rows.get(screen.nav.cursor) {
        Some(row) => format!("{}/{total}", row.number),
        None => "0/0".to_string(),
    };
    let chrome = Chrome {
        left,
        right: app.right_segments(now),
        hints: hints(screen, directory),
        position,
    };
    let inner = draw_frame(frame, frame.area(), &chrome, theme, edge);
    draw_rows(frame.buffer_mut(), inner, screen, &rows, total, theme);
}

/// The prompt, the number being typed, the moving keys, or the screen's keys.
fn hints(screen: &NodesScreen, directory: Option<&[Node]>) -> Vec<(String, String)> {
    match &screen.prompt {
        Some(prompt) if prompt.label == GOTO => return goto_hints(prompt, directory),
        Some(prompt) => return vec![prompt.hint()],
        None => {}
    }
    if !screen.nav.typed.is_empty() {
        return vec![hint("Enter", &format!("跳到 {}", screen.nav.typed))];
    }
    if screen.moving {
        return vec![hint("↑↓", "移动位置"), hint("M", "完成")];
    }
    let other = match screen.view {
        NodesView::Favorites => "全部节点",
        NodesView::All => "我的最爱",
    };
    let filter = match &screen.filter {
        Some(filter) => format!("过滤：{filter}"),
        None => "过滤".to_string(),
    };
    vec![
        hint("↑↓", "移动"),
        hint("→", "进入"),
        hint("y", other),
        hint("/", &filter),
        hint("?", "说明"),
    ]
}

/// One row per line from the top of the page, with a column of space inside each side of the
/// border: the cursor, the number, the name, the display name, and the topic count at the right.
fn draw_rows(
    buf: &mut Buffer,
    inner: Rect,
    screen: &NodesScreen,
    rows: &[Row],
    total: usize,
    theme: &Theme,
) {
    let area = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    let digits = total.to_string().len();
    let page = rows.iter().enumerate().skip(screen.nav.offset);
    for (y, (i, row)) in (area.top()..area.bottom()).zip(page) {
        let selected = i == screen.nav.cursor;
        let count = row.topics.map_or(String::new(), |n| format!(" {n}"));
        let count = Line::styled(count, theme.style(Paint::Minor));
        // A topic count is at most 11 columns.
        let count_width = (count.width() as u16).min(area.width);
        let title = if selected {
            Paint::Selected
        } else {
            Paint::Plain
        };
        let line = Line::from(vec![
            Span::styled(
                if selected { "❯ " } else { "  " },
                theme.style(Paint::Cursor),
            ),
            Span::styled(
                format!("{:>digits$} ", row.number),
                theme.style(Paint::Minor),
            ),
            Span::styled(sanitize(&row.name), theme.style(Paint::Location)),
            Span::raw("  "),
            Span::styled(sanitize(&row.title), theme.style(title)),
        ]);
        buf.set_line(area.x, y, &line, area.width - count_width);
        buf.set_line(area.right() - count_width, y, &count, count_width);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use v2ex_rs::Error;

    use super::*;
    use crate::store::favorites::Favorite;
    use crate::tui::Start;
    use crate::tui::app::testing::*;
    use crate::tui::app::{Msg, Screen};

    fn shown(app: &App) -> Vec<Row> {
        nodes_screen(app).rows(&app.favorites, app.directory.as_deref())
    }

    fn titles(app: &App) -> Vec<String> {
        shown(app).into_iter().map(|row| row.title).collect()
    }

    fn cursor(app: &App) -> usize {
        nodes_screen(app).nav.cursor
    }

    fn favorite_titles(app: &App) -> Vec<&str> {
        app.favorites
            .items()
            .iter()
            .map(|f| f.title.as_str())
            .collect()
    }

    /// An app on the all-nodes view with `directory` loaded.
    fn all_nodes(directory: Vec<Node>) -> App {
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('y'));
        load_directory(&mut app, directory);
        app
    }

    #[test]
    fn rows_number_the_favorites_and_the_feeds_tabs_and_nodes() {
        let favorites = Favorites::load(None);
        let screen = NodesScreen::default();
        let rows = screen.rows(&favorites, None);
        assert_eq!(rows.len(), 7);
        assert_eq!(
            rows[2],
            Row {
                number: 3,
                listing: Listing::Tab("tech".into()),
                name: "tech".into(),
                title: "技术".into(),
                topics: None,
            }
        );
        assert_eq!(
            (rows[0].name.as_str(), rows[1].name.as_str()),
            ("hot", "latest")
        );

        let screen = NodesScreen {
            view: NodesView::All,
            ..NodesScreen::default()
        };
        let rows = screen.rows(&favorites, None);
        let names: Vec<_> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "hot", "latest", "tech", "creative", "play", "apple", "jobs", "deals", "city",
                "qna", "all"
            ]
        );
        assert_eq!(rows[5].title, "Apple");

        let directory = [node("go", "Go", 5), node("python", "Python", 50)];
        let rows = screen.rows(&favorites, Some(&directory));
        assert_eq!(rows.len(), 13);
        assert_eq!(
            rows[11],
            Row {
                number: 12,
                listing: Listing::Node("python".into()),
                name: "python".into(),
                title: "Python".into(),
                topics: Some(50),
            }
        );
        assert_eq!(rows[12].name, "go");

        // The filter matches names and titles, ignoring case.
        let screen = NodesScreen {
            view: NodesView::All,
            filter: Some("PY".into()),
            ..NodesScreen::default()
        };
        let rows = screen.rows(&favorites, Some(&directory));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number, 12);
        let screen = NodesScreen {
            view: NodesView::All,
            filter: Some("appl".into()),
            ..NodesScreen::default()
        };
        assert_eq!(screen.rows(&favorites, Some(&directory))[0].name, "apple");
    }

    #[test]
    fn y_switches_to_all_nodes_and_loads_the_directory_once() {
        let (mut app, _) = launch(Start::Home);
        let cmds = code(&mut app, KeyCode::Char('y'));
        assert_eq!(cmds, [Cmd::LoadDirectory { refresh: false }]);
        assert_eq!(nodes_screen(&app).view, NodesView::All);
        assert!(app.directory_loading);
        // Leaving and coming back while it loads asks only once.
        assert!(chars(&mut app, "yy").is_empty());

        let cmds = load_directory(
            &mut app,
            vec![node("go", "Go", 5), node("python", "Python", 50)],
        );
        assert!(cmds.is_empty());
        assert!(!app.directory_loading);
        let rows = shown(&app);
        assert_eq!(rows.len(), 13);
        assert_eq!(rows[11].name, "python");
        assert_eq!(rows[12].name, "go");

        assert!(code(&mut app, KeyCode::Char('y')).is_empty());
        assert_eq!(nodes_screen(&app).view, NodesView::Favorites);
        assert_eq!(shown(&app).len(), 7);
        assert!(code(&mut app, KeyCode::Char('y')).is_empty());
        assert_eq!(nodes_screen(&app).view, NodesView::All);
    }

    #[test]
    fn r_reloads_the_directory_in_the_all_nodes_view_only() {
        let (mut app, _) = launch(Start::Home);
        assert!(code(&mut app, KeyCode::Char('r')).is_empty());
        let mut app = all_nodes(nodes(3));
        assert_eq!(
            code(&mut app, KeyCode::Char('r')),
            [Cmd::LoadDirectory { refresh: true }]
        );
        assert!(app.directory_loading);
    }

    #[test]
    fn r_does_not_ask_again_while_the_directory_is_on_its_way() {
        let mut app = all_nodes(nodes(3));
        code(&mut app, KeyCode::Char('r'));
        assert!(code(&mut app, KeyCode::Char('r')).is_empty());
        load_directory(&mut app, nodes(4));
        assert_eq!(
            code(&mut app, KeyCode::Char('r')),
            [Cmd::LoadDirectory { refresh: true }]
        );

        // The first load counts too.
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('y'));
        assert!(code(&mut app, KeyCode::Char('r')).is_empty());
    }

    #[test]
    fn enter_opens_the_row_under_the_cursor() {
        let mut app = all_nodes(nodes(3));
        code(&mut app, KeyCode::End);
        let cmds = code(&mut app, KeyCode::Char('l'));
        let Some(Screen::Topics(topics)) = app.stack.last() else {
            panic!("expected Topics on top");
        };
        assert_eq!(topics.listing, Listing::Node("node1".into()));
        assert_eq!(topics.title, "节点1");
        assert_eq!(
            cmds,
            [Cmd::LoadTopics {
                ticket: topics.pending.unwrap(),
                listing: Listing::Node("node1".into()),
                refresh: false
            }]
        );
    }

    #[test]
    fn add_remove_and_move_change_favorites_and_ask_for_a_save() {
        let mut app = all_nodes(vec![node("python", "Python", 50)]);
        code(&mut app, KeyCode::End);
        assert_eq!(code(&mut app, KeyCode::Char('a')), [Cmd::SaveFavorites]);
        assert_eq!(app.favorites.items().len(), 8);
        assert_eq!(
            app.favorites.items()[7],
            Favorite {
                listing: Listing::Node("python".into()),
                title: "Python".into()
            }
        );
        assert!(code(&mut app, KeyCode::Char('a')).is_empty());
        assert_eq!(app.favorites.items().len(), 8);
        // d and M do nothing in the all-nodes view.
        assert!(chars(&mut app, "dM").is_empty());
        assert_eq!(app.favorites.items().len(), 8);
        assert!(!nodes_screen(&app).moving);

        code(&mut app, KeyCode::Char('y'));
        assert_eq!(nodes_screen(&app).view, NodesView::Favorites);
        // a does nothing in the favorites view.
        assert!(code(&mut app, KeyCode::Char('a')).is_empty());
        assert_eq!(code(&mut app, KeyCode::Char('d')), [Cmd::SaveFavorites]);
        assert_eq!(
            favorite_titles(&app),
            ["最新", "技术", "创意", "好玩", "问与答", "全部", "Python"]
        );

        // Removing the last row keeps the cursor on the list.
        code(&mut app, KeyCode::End);
        assert_eq!(code(&mut app, KeyCode::Char('d')), [Cmd::SaveFavorites]);
        assert_eq!(app.favorites.items().len(), 6);
        assert_eq!(cursor(&app), 5);

        code(&mut app, KeyCode::Home);
        assert!(code(&mut app, KeyCode::Char('M')).is_empty());
        assert!(nodes_screen(&app).moving);
        assert_eq!(code(&mut app, KeyCode::Down), [Cmd::SaveFavorites]);
        assert_eq!(cursor(&app), 1);
        assert_eq!(code(&mut app, KeyCode::Char('j')), [Cmd::SaveFavorites]);
        assert_eq!(
            favorite_titles(&app),
            ["技术", "创意", "最新", "好玩", "问与答", "全部"]
        );
        assert_eq!(code(&mut app, KeyCode::Char('k')), [Cmd::SaveFavorites]);
        assert_eq!(
            favorite_titles(&app),
            ["技术", "最新", "创意", "好玩", "问与答", "全部"]
        );
        // Other keys do nothing while moving.
        assert!(chars(&mut app, "dqy/").is_empty());
        assert_eq!(app.favorites.items().len(), 6);
        assert!(code(&mut app, KeyCode::Char('M')).is_empty());
        assert!(!nodes_screen(&app).moving);
        assert!(!app.quit);

        // The first row cannot move up, the last cannot move down; Enter and Esc end moving.
        code(&mut app, KeyCode::Home);
        code(&mut app, KeyCode::Char('M'));
        assert!(code(&mut app, KeyCode::Up).is_empty());
        assert!(code(&mut app, KeyCode::Enter).is_empty());
        assert!(!nodes_screen(&app).moving);
        assert_eq!(app.stack.len(), 1);
        code(&mut app, KeyCode::End);
        code(&mut app, KeyCode::Char('M'));
        assert!(code(&mut app, KeyCode::Down).is_empty());
        assert!(code(&mut app, KeyCode::Esc).is_empty());
        assert!(!nodes_screen(&app).moving);
        assert_eq!(
            favorite_titles(&app),
            ["技术", "最新", "创意", "好玩", "问与答", "全部"]
        );
    }

    #[test]
    fn filter_narrows_rows_keeps_numbers_and_back_clears_it_first() {
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('/'));
        assert!(nodes_screen(&app).prompt.is_some());
        code(&mut app, KeyCode::Char('技'));
        // The rows narrow while typing.
        assert_eq!(titles(&app), ["技术"]);
        code(&mut app, KeyCode::Enter);
        assert!(nodes_screen(&app).prompt.is_none());
        let rows = shown(&app);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "技术");
        assert_eq!(rows[0].number, 3);
        assert_eq!(cursor(&app), 0);

        assert!(chars(&mut app, "1").is_empty());
        assert!(code(&mut app, KeyCode::Enter).is_empty());
        assert_eq!(cursor(&app), 0);
        assert_eq!(app.stack.len(), 1);
        chars(&mut app, "3");
        code(&mut app, KeyCode::Enter);
        assert_eq!(cursor(&app), 0);

        assert!(code(&mut app, KeyCode::Left).is_empty());
        assert!(nodes_screen(&app).filter.is_none());
        assert_eq!(shown(&app).len(), 7);
        // The cursor stays on the row it was on.
        assert_eq!(cursor(&app), 2);
        assert_eq!(app.stack.len(), 1);
        assert!(!app.quit);
        code(&mut app, KeyCode::Left);
        assert!(app.quit);
    }

    #[test]
    fn esc_cancels_the_filter_and_an_empty_filter_is_none() {
        let (mut app, _) = launch(Start::Home);
        chars(&mut app, "/全");
        assert_eq!(titles(&app), ["全部"]);
        code(&mut app, KeyCode::Esc);
        assert!(nodes_screen(&app).prompt.is_none());
        assert!(nodes_screen(&app).filter.is_none());
        assert_eq!(cursor(&app), 6);

        // ← in the prompt cancels it like Esc, without leaving the screen.
        code(&mut app, KeyCode::Home);
        chars(&mut app, "/全");
        assert!(code(&mut app, KeyCode::Left).is_empty());
        assert!(nodes_screen(&app).prompt.is_none());
        assert!(nodes_screen(&app).filter.is_none());
        assert_eq!(shown(&app).len(), 7);
        assert_eq!(cursor(&app), 6);
        assert!(!app.quit);

        chars(&mut app, "/x");
        code(&mut app, KeyCode::Backspace);
        code(&mut app, KeyCode::Enter);
        assert!(nodes_screen(&app).filter.is_none());
        assert_eq!(shown(&app).len(), 7);

        // Keys that are commands elsewhere are text in the prompt.
        chars(&mut app, "/q?yh");
        assert_eq!(nodes_screen(&app).filter.as_deref(), Some("q?yh"));
        assert_eq!(app.stack.len(), 1);
        assert_eq!(nodes_screen(&app).view, NodesView::Favorites);
        assert!(!app.quit);
    }

    #[test]
    fn a_number_then_enter_moves_to_that_row() {
        let mut app = all_nodes(nodes(100));
        chars(&mut app, "42");
        assert!(code(&mut app, KeyCode::Enter).is_empty());
        assert_eq!(cursor(&app), 41);
        assert_eq!(shown(&app)[41].number, 42);
        let nav = &nodes_screen(&app).nav;
        assert!(nav.offset <= 41 && 41 < nav.offset + 22);

        // No row 0 or 999: the cursor stays.
        for number in ["0", "999"] {
            chars(&mut app, number);
            code(&mut app, KeyCode::Enter);
            assert_eq!(cursor(&app), 41);
        }
        assert_eq!(app.stack.len(), 1);
    }

    #[test]
    fn a_filter_with_no_match_leaves_an_empty_list_that_ignores_keys() {
        for view_key in ["", "y"] {
            let (mut app, _) = launch(Start::Home);
            chars(&mut app, view_key);
            chars(&mut app, "/zzz");
            code(&mut app, KeyCode::Enter);
            assert!(shown(&app).is_empty());
            for key in [
                KeyCode::Char('j'),
                KeyCode::Char('k'),
                KeyCode::End,
                KeyCode::PageDown,
                KeyCode::Enter,
                KeyCode::Right,
                KeyCode::Char('a'),
                KeyCode::Char('d'),
                KeyCode::Char('M'),
                KeyCode::Down,
            ] {
                assert!(code(&mut app, key).is_empty(), "{key:?}");
                assert_eq!(app.stack.len(), 1);
                assert!(!nodes_screen(&app).moving);
                assert_eq!((cursor(&app), nodes_screen(&app).nav.offset), (0, 0));
            }
            chars(&mut app, "5");
            assert!(code(&mut app, KeyCode::Enter).is_empty());
            assert_eq!(app.favorites.items().len(), 7);
        }
    }

    #[test]
    fn a_directory_error_keeps_the_view_and_sets_the_error() {
        let (mut app, _) = launch(Start::Home);
        code(&mut app, KeyCode::Char('y'));
        let cmds = app.update(
            Msg::DirectoryLoaded {
                result: Err(Error::Network("连接超时".into())),
                quota: None,
            },
            now(),
        );
        assert!(cmds.is_empty());
        assert!(!app.directory_loading);
        assert!(app.error.as_deref().unwrap().contains("连接超时"));
        assert_eq!(shown(&app).len(), 11);
        assert_eq!(nodes_screen(&app).view, NodesView::All);

        // A later success clears the error; a failed reload keeps the nodes.
        load_directory(&mut app, nodes(4));
        assert!(app.error.is_none());
        code(&mut app, KeyCode::Char('r'));
        app.update(
            Msg::DirectoryLoaded {
                result: Err(Error::Status(502)),
                quota: None,
            },
            now(),
        );
        assert!(app.error.is_some());
        assert_eq!(shown(&app).len(), 15);
    }

    #[test]
    fn the_directory_brings_the_quota_and_a_shorter_one_pulls_the_cursor_back() {
        let mut app = all_nodes(nodes(50));
        code(&mut app, KeyCode::End);
        assert_eq!(cursor(&app), 60);
        let quota = v2ex_rs::RateLimit {
            limit: 600,
            remaining: 590,
            reset: now(),
        };
        app.update(
            Msg::DirectoryLoaded {
                result: Ok(nodes(5)),
                quota: Some(quota.clone()),
            },
            now(),
        );
        assert_eq!(app.quota, Some(quota));
        assert_eq!(cursor(&app), 15);
        let nav = &nodes_screen(&app).nav;
        assert!(nav.offset <= nav.cursor && nav.cursor < nav.offset + 22);
    }
}
