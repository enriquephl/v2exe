//! The one part of the interface that touches the outside world: it owns the terminal, turns
//! its events into [`Msg`], and carries out every [`Cmd`]: requests in tasks of their own (the
//! directory's task also writes `nodes.json`), the favorites and read-state saves and the
//! browser in place.

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use chrono::Utc;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use v2ex_rs::{Client, Node};

use super::Options;
use super::app::{App, Cmd, Msg, view};
use super::theme::Theme;
use crate::store::favorites::Favorites;
use crate::store::node_cache::{load_nodes, save_nodes};
use crate::store::paths::Dirs;
use crate::store::read_state::ReadState;

/// What a background task that panicked comes back as.
const TASK_PANICKED: &str = "后台任务意外中止";

/// Runs the interface until the user quits. The terminal is in raw mode on the alternate screen
/// meanwhile, and is restored before this returns and before a panic is reported. A panic in a
/// background task, or a read of the terminal that fails, ends the interface with an error.
///
/// The favorites, the read state and the node directory are read before the terminal is
/// switched, so a file whose read blocks (a FIFO, say) holds up a terminal that is still in its
/// normal mode, where Ctrl-C ends the program.
pub async fn run(client: Client, options: Options) -> io::Result<()> {
    let files = Files::load(&options.dirs);
    let mut terminal = ratatui::try_init().inspect_err(|_| ratatui::restore())?;
    let result = run_in(&mut terminal, client, options, files).await;
    // The panic hook has restored the terminal when raw mode is already off. Restoring again
    // would leave the alternate screen twice, which moves the cursor back over the panic report.
    if raw_mode() {
        ratatui::restore();
    }
    result
}

/// Whether the terminal is still in the raw mode `run` entered. Only a restore turns it off, and
/// while the loop runs only the panic hook restores: raw mode off then means something panicked.
fn raw_mode() -> bool {
    crossterm::terminal::is_raw_mode_enabled().unwrap_or(true)
}

/// What the interface starts from on disk.
struct Files {
    favorites: Favorites,
    read: ReadState,
    /// `nodes.json` in the cache directory, when there is one.
    nodes_path: Option<PathBuf>,
    /// The node directory in `nodes.json`, while it is fresh.
    directory: Option<Vec<Node>>,
}

impl Files {
    fn load(dirs: &Dirs) -> Files {
        let state = |name: &str| dirs.state.as_ref().map(|dir| dir.join(name));
        let nodes_path = dirs.cache.as_ref().map(|dir| dir.join("nodes.json"));
        Files {
            favorites: Favorites::load(state("favorites.json")),
            read: ReadState::load(state("read.json")),
            directory: nodes_path
                .as_deref()
                .and_then(|path| load_nodes(path, Utc::now())),
            nodes_path,
        }
    }
}

/// Draws a frame, waits for a message, updates the app with it and carries out its commands,
/// until the app quits. Stops with an error once a panic has restored the terminal, before
/// drawing over the report.
async fn run_in(
    terminal: &mut DefaultTerminal,
    client: Client,
    options: Options,
    files: Files,
) -> io::Result<()> {
    let Options {
        start,
        edge,
        color,
        tz,
        dirs: _,
    } = options;
    let size = terminal.size()?;
    let (mut app, cmds) = App::new(
        start,
        files.favorites,
        files.read,
        (size.width, size.height),
        tz,
    );

    let (tx, mut rx) = unbounded_channel();
    let runner = Runner {
        client,
        tx: tx.clone(),
        nodes_path: files.nodes_path,
    };
    if let Some(nodes) = files.directory {
        runner.send(Msg::DirectoryLoaded {
            result: Ok(nodes),
            quota: None,
        });
    }
    for cmd in cmds {
        runner.execute(cmd, &app);
    }
    forward_input(tx);

    let theme = Theme { color };
    while !app.quit {
        if !raw_mode() {
            return Err(io::Error::other(TASK_PANICKED));
        }
        terminal.draw(|frame| view(&app, &theme, edge, Utc::now(), frame))?;
        let Some(next) = rx.recv().await else {
            break;
        };
        for cmd in app.update(next?, Utc::now()) {
            runner.execute(cmd, &app);
        }
    }
    Ok(())
}

/// Reads the terminal in a thread of its own, which blocks on each read, and passes what it reads
/// to [`forward`].
fn forward_input(tx: UnboundedSender<io::Result<Msg>>) {
    thread::spawn(move || forward(std::iter::repeat_with(event::read), tx));
}

/// Sends the key presses and every new terminal size among `events`, until the interface is
/// gone. A read that fails is sent as the error that ends the loop, and ends the forwarding.
fn forward(
    events: impl IntoIterator<Item = io::Result<Event>>,
    tx: UnboundedSender<io::Result<Msg>>,
) {
    for event in events {
        let msg = match event {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => Msg::Key(key),
            Ok(Event::Resize(columns, rows)) => Msg::Resize(columns, rows),
            Ok(_) => continue,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        if tx.send(Ok(msg)).is_err() {
            return;
        }
    }
}

/// Carries out commands. A request runs in a task of its own and its result comes back as a
/// message; the directory's task also writes `nodes.json`. Saving the favorites or the read
/// state, and opening the browser, run in place, and only a failure comes back. A task that
/// panics comes back as an error, which ends the loop.
struct Runner {
    client: Client,
    tx: UnboundedSender<io::Result<Msg>>,
    /// `nodes.json` in the cache directory, when there is one.
    nodes_path: Option<PathBuf>,
}

impl Runner {
    fn execute(&self, cmd: Cmd, app: &App) {
        match cmd {
            Cmd::LoadTopics {
                ticket,
                listing,
                refresh,
            } => {
                let client = self.client_for(refresh);
                self.spawn(async move {
                    let result = client.topics(&listing, 1).await.map(|page| page.items);
                    vec![Msg::TopicsLoaded {
                        ticket,
                        result,
                        quota: client.rate_limit(),
                    }]
                });
            }
            Cmd::LoadThread {
                ticket,
                id,
                refresh,
            } => {
                let client = self.client_for(refresh);
                self.spawn(async move {
                    // The replies carry no node, so the topic comes first: it is the one refused
                    // when its node is restricted.
                    let result = match client.topic(id).await {
                        Ok(topic) => client.replies(id, 1).await.map(|page| (topic, page.items)),
                        Err(e) => Err(e),
                    };
                    vec![Msg::ThreadLoaded {
                        ticket,
                        result,
                        quota: client.rate_limit(),
                    }]
                });
            }
            Cmd::LoadDirectory { refresh } => self.load_directory(refresh),
            Cmd::OpenUrl(url) => {
                if let Err(reason) = open_url(&url) {
                    self.send(Msg::SaveFailed(reason));
                }
            }
            Cmd::SaveFavorites => {
                if let Err(e) = app.favorites.save() {
                    self.send(Msg::SaveFailed(format!("无法写入 favorites.json：{e}")));
                }
            }
            Cmd::SaveRead => {
                if let Err(e) = app.read.save() {
                    self.send(Msg::SaveFailed(format!("无法写入 read.json：{e}")));
                }
            }
        }
    }

    /// The node directory in `nodes.json`, while it is fresh.
    fn saved_directory(&self) -> Option<Vec<Node>> {
        load_nodes(self.nodes_path.as_deref()?, Utc::now())
    }

    /// Takes the directory from `nodes.json` while it is fresh, unless `refresh`; otherwise asks
    /// the site and keeps what it returns in `nodes.json`.
    fn load_directory(&self, refresh: bool) {
        if !refresh && let Some(nodes) = self.saved_directory() {
            self.send(Msg::DirectoryLoaded {
                result: Ok(nodes),
                quota: None,
            });
            return;
        }
        let client = self.client_for(refresh);
        let path = self.nodes_path.clone();
        self.spawn(async move {
            let result = client.nodes().await;
            let saved = match (&result, &path) {
                (Ok(nodes), Some(path)) => save_nodes(path, nodes, Utc::now()),
                _ => Ok(()),
            };
            let mut msgs = vec![Msg::DirectoryLoaded {
                result,
                quota: client.rate_limit(),
            }];
            if let Err(e) = saved {
                msgs.push(Msg::SaveFailed(format!("无法写入 nodes.json：{e}")));
            }
            msgs
        });
    }

    /// The client, with every cached response dropped first when `refresh`.
    fn client_for(&self, refresh: bool) -> Client {
        if refresh {
            self.client.clear_cache();
        }
        self.client.clone()
    }

    /// Runs `task` in a task of its own and sends the messages it ends with, in order. Every
    /// background task goes through here: when one panics, tokio keeps the panic in the task,
    /// so this sends it on as the error that ends the loop.
    fn spawn(&self, task: impl Future<Output = Vec<Msg>> + Send + 'static) {
        let tx = self.tx.clone();
        let task = tokio::spawn(task);
        tokio::spawn(async move {
            match task.await {
                Ok(msgs) => {
                    for msg in msgs {
                        let _ = tx.send(Ok(msg));
                    }
                }
                Err(_) => {
                    let _ = tx.send(Err(io::Error::other(TASK_PANICKED)));
                }
            }
        });
    }

    /// The receiver lives as long as the runner, so the message always arrives.
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(Ok(msg));
    }
}

/// Opens `url` in the browser without waiting for it to finish. Only `http://` and `https://`
/// addresses are opened.
fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只打开 http 或 https 网址".to_string());
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("无法打开浏览器：{e}"))?;
    // Waited for aside, so a finished opener leaves no zombie process.
    thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio::sync::mpsc::error::TryRecvError;
    use v2ex_rs::{Error, Listing};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::tui::Start;
    use crate::tui::app::testing::{launch, node, tz};

    const TOPICS: &str = include_str!("../../../v2ex-rs/tests/fixtures/api1/topics.json");
    const TOPIC: &str = include_str!("../../../v2ex-rs/tests/fixtures/api1/topic.json");
    const TOPIC_RESTRICTED: &str =
        include_str!("../../../v2ex-rs/tests/fixtures/api1/topic_restricted.json");
    const REPLIES: &str = include_str!("../../../v2ex-rs/tests/fixtures/api1/replies.json");
    const NODES: &str = include_str!("../../../v2ex-rs/tests/fixtures/api1/nodes.json");

    fn serve(path_: &str, query: Option<(&str, &str)>, body: &str, times: u64) -> Mock {
        let mut matcher = Mock::given(method("GET")).and(path(path_));
        if let Some((key, value)) = query {
            matcher = matcher.and(query_param(key, value));
        }
        matcher
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(times)
    }

    fn runner(
        server: &MockServer,
        nodes_path: Option<PathBuf>,
    ) -> (Runner, UnboundedReceiver<io::Result<Msg>>) {
        let client = Client::builder().base_url(server.uri()).build().unwrap();
        let (tx, rx) = unbounded_channel();
        let runner = Runner {
            client,
            tx,
            nodes_path,
        };
        (runner, rx)
    }

    fn key(c: char, kind: KeyEventKind) -> io::Result<Event> {
        Ok(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char(c),
            KeyModifiers::NONE,
            kind,
        )))
    }

    #[test]
    fn input_forwards_key_presses_and_sizes_only() {
        let (tx, mut rx) = unbounded_channel();

        forward(
            [
                key('a', KeyEventKind::Press),
                key('b', KeyEventKind::Release),
                key('c', KeyEventKind::Repeat),
                Ok(Event::FocusGained),
                Ok(Event::Resize(100, 30)),
            ],
            tx,
        );

        let Ok(Ok(Msg::Key(pressed))) = rx.try_recv() else {
            panic!("expected the key press");
        };
        assert_eq!(pressed.code, KeyCode::Char('a'));
        assert!(matches!(rx.try_recv(), Ok(Ok(Msg::Resize(100, 30)))));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn a_read_error_is_forwarded_and_ends_the_input() {
        let (tx, mut rx) = unbounded_channel();

        forward(
            [
                key('a', KeyEventKind::Press),
                Err(io::Error::other("示例：读不到终端")),
                key('b', KeyEventKind::Press),
            ],
            tx,
        );

        assert!(matches!(rx.try_recv(), Ok(Ok(Msg::Key(_)))));
        let Ok(Err(e)) = rx.try_recv() else {
            panic!("expected the read error");
        };
        assert_eq!(e.to_string(), "示例：读不到终端");
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[tokio::test]
    async fn a_task_that_panics_comes_back_as_the_error_that_ends_the_loop() {
        let server = MockServer::start().await;
        let (runner, mut rx) = runner(&server, None);

        runner.spawn(async { panic!("示例：解析时出错") });

        let Some(Err(e)) = rx.recv().await else {
            panic!("expected an error");
        };
        assert_eq!(e.to_string(), "后台任务意外中止");
        drop(runner);
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn a_task_sends_its_messages_in_order() {
        let server = MockServer::start().await;
        let (runner, mut rx) = runner(&server, None);

        runner.spawn(async { vec![Msg::SaveFailed("一".into()), Msg::SaveFailed("二".into())] });

        for want in ["一", "二"] {
            let Some(Ok(Msg::SaveFailed(reason))) = rx.recv().await else {
                panic!("expected SaveFailed");
            };
            assert_eq!(reason, want);
        }
    }

    /// Loads the hot listing twice through one runner, the second time with `refresh`, from a
    /// site that expects `requests` requests for it.
    async fn load_hot_twice(refresh: bool, requests: u64) {
        let server = MockServer::start().await;
        serve("/api/topics/hot.json", None, TOPICS, requests)
            .mount(&server)
            .await;
        let (runner, mut rx) = runner(&server, None);
        let (app, _) = launch(Start::Home);

        for (ticket, refresh) in [(1, false), (2, refresh)] {
            runner.execute(
                Cmd::LoadTopics {
                    ticket,
                    listing: Listing::Hot,
                    refresh,
                },
                &app,
            );
            let Some(Ok(Msg::TopicsLoaded { result, .. })) = rx.recv().await else {
                panic!("expected TopicsLoaded");
            };
            assert!(result.is_ok(), "{result:?}");
        }

        server.verify().await;
    }

    #[tokio::test]
    async fn a_refresh_asks_the_site_again() {
        load_hot_twice(true, 2).await;
    }

    #[tokio::test]
    async fn without_a_refresh_the_cached_listing_is_reused() {
        load_hot_twice(false, 1).await;
    }

    #[tokio::test]
    async fn a_thread_is_its_topic_then_its_replies() {
        let server = MockServer::start().await;
        serve("/api/topics/show.json", Some(("id", "1001")), TOPIC, 1)
            .mount(&server)
            .await;
        serve(
            "/api/replies/show.json",
            Some(("topic_id", "1001")),
            REPLIES,
            1,
        )
        .mount(&server)
        .await;
        let (runner, mut rx) = runner(&server, None);
        let (app, _) = launch(Start::Home);

        runner.execute(
            Cmd::LoadThread {
                ticket: 7,
                id: 1001,
                refresh: false,
            },
            &app,
        );

        let Some(Ok(Msg::ThreadLoaded { ticket, result, .. })) = rx.recv().await else {
            panic!("expected ThreadLoaded");
        };
        let (topic, replies) = result.unwrap();
        assert_eq!((ticket, topic.id, replies.len()), (7, 1001, 2));
    }

    #[tokio::test]
    async fn a_refused_topic_is_not_followed_by_its_replies() {
        let server = MockServer::start().await;
        serve(
            "/api/topics/show.json",
            Some(("id", "1002")),
            TOPIC_RESTRICTED,
            1,
        )
        .mount(&server)
        .await;
        serve("/api/replies/show.json", None, REPLIES, 0)
            .mount(&server)
            .await;
        let (runner, mut rx) = runner(&server, None);
        let (app, _) = launch(Start::Home);

        runner.execute(
            Cmd::LoadThread {
                ticket: 1,
                id: 1002,
                refresh: false,
            },
            &app,
        );

        let Some(Ok(Msg::ThreadLoaded { result, .. })) = rx.recv().await else {
            panic!("expected ThreadLoaded");
        };
        assert!(
            matches!(result, Err(Error::Restricted { .. })),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn the_directory_comes_from_a_fresh_nodes_json_until_a_refresh() {
        let server = MockServer::start().await;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cache").join("nodes.json");
        save_nodes(&path, &[node("saved", "保存的节点", 1)], Utc::now()).unwrap();
        let (runner, mut rx) = runner(&server, Some(path.clone()));
        let (app, _) = launch(Start::Home);

        serve("/api/nodes/all.json", None, NODES, 0)
            .mount(&server)
            .await;
        runner.execute(Cmd::LoadDirectory { refresh: false }, &app);
        let Some(Ok(Msg::DirectoryLoaded { result, quota })) = rx.recv().await else {
            panic!("expected DirectoryLoaded");
        };
        assert_eq!(result.unwrap()[0].name, "saved");
        assert_eq!(quota, None);
        server.verify().await;
        server.reset().await;

        serve("/api/nodes/all.json", None, NODES, 1)
            .mount(&server)
            .await;
        runner.execute(Cmd::LoadDirectory { refresh: true }, &app);
        let Some(Ok(Msg::DirectoryLoaded { result, .. })) = rx.recv().await else {
            panic!("expected DirectoryLoaded");
        };
        let names: Vec<String> = result.unwrap().into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["python", "qna"]);
        let saved: Vec<String> = load_nodes(&path, Utc::now())
            .unwrap()
            .into_iter()
            .map(|n| n.name)
            .collect();
        assert_eq!(saved, names);
    }

    #[tokio::test]
    async fn a_failed_write_comes_back_as_a_message() {
        let server = MockServer::start().await;
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("state");
        std::fs::write(&blocker, "a file, not a directory").unwrap();
        let (runner, mut rx) = runner(&server, None);
        let (app, _) = App::new(
            Start::Home,
            Favorites::load(Some(blocker.join("favorites.json"))),
            ReadState::load(Some(blocker.join("read.json"))),
            (80, 24),
            tz(),
        );

        runner.execute(Cmd::SaveFavorites, &app);
        runner.execute(Cmd::SaveRead, &app);
        runner.execute(Cmd::OpenUrl("file:///etc/hosts".into()), &app);

        for name in ["favorites.json", "read.json", "http"] {
            let Some(Ok(Msg::SaveFailed(reason))) = rx.recv().await else {
                panic!("expected SaveFailed");
            };
            assert!(reason.contains(name), "{name}: {reason}");
        }
        assert!(rx.try_recv().is_err());
    }
}
