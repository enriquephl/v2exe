//! The one part of the interface that touches the outside world: it owns the terminal, turns
//! its events into [`Msg`], and carries out every [`Cmd`]: requests in tasks of their own, file
//! writes and the browser in place.

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
use crate::store::read_state::ReadState;

/// Runs the interface until the user quits. The terminal is in raw mode on the alternate screen
/// meanwhile, and is restored before this returns and before a panic is reported.
pub async fn run(client: Client, options: Options) -> io::Result<()> {
    let mut terminal = ratatui::try_init().inspect_err(|_| ratatui::restore())?;
    let result = run_in(&mut terminal, client, options).await;
    ratatui::restore();
    result
}

/// Draws a frame, waits for a message, updates the app with it and carries out its commands,
/// until the app quits.
async fn run_in(
    terminal: &mut DefaultTerminal,
    client: Client,
    options: Options,
) -> io::Result<()> {
    let Options {
        start,
        edge,
        color,
        tz,
        dirs,
    } = options;
    let state = |name: &str| dirs.state.as_ref().map(|dir| dir.join(name));
    let size = terminal.size()?;
    let (mut app, cmds) = App::new(
        start,
        Favorites::load(state("favorites.json")),
        ReadState::load(state("read.json")),
        (size.width, size.height),
        tz,
    );

    let (tx, mut rx) = unbounded_channel();
    let runner = Runner {
        client,
        tx: tx.clone(),
        nodes_path: dirs.cache.map(|dir| dir.join("nodes.json")),
    };
    if let Some(nodes) = runner.saved_directory() {
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
        terminal.draw(|frame| view(&app, &theme, edge, Utc::now(), frame))?;
        let Some(msg) = rx.recv().await else {
            break;
        };
        for cmd in app.update(msg, Utc::now()) {
            runner.execute(cmd, &app);
        }
    }
    Ok(())
}

/// Sends key presses and every new terminal size from a thread of its own, which blocks reading
/// the terminal. The thread ends when the terminal cannot be read or the interface is gone.
fn forward_input(tx: UnboundedSender<Msg>) {
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            let msg = match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => Msg::Key(key),
                Event::Resize(columns, rows) => Msg::Resize(columns, rows),
                _ => continue,
            };
            if tx.send(msg).is_err() {
                break;
            }
        }
    });
}

/// Carries out commands. A request runs in a task of its own and its result comes back as a
/// message; a file write or the browser runs in place, and only a failure comes back.
struct Runner {
    client: Client,
    tx: UnboundedSender<Msg>,
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
                    Msg::TopicsLoaded {
                        ticket,
                        result,
                        quota: client.rate_limit(),
                    }
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
                    Msg::ThreadLoaded {
                        ticket,
                        result,
                        quota: client.rate_limit(),
                    }
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
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = client.nodes().await;
            let saved = match (&result, &path) {
                (Ok(nodes), Some(path)) => save_nodes(path, nodes, Utc::now()),
                _ => Ok(()),
            };
            let _ = tx.send(Msg::DirectoryLoaded {
                result,
                quota: client.rate_limit(),
            });
            if let Err(e) = saved {
                let _ = tx.send(Msg::SaveFailed(format!("无法写入 nodes.json：{e}")));
            }
        });
    }

    /// The client, with every cached response dropped first when `refresh`.
    fn client_for(&self, refresh: bool) -> Client {
        if refresh {
            self.client.clear_cache();
        }
        self.client.clone()
    }

    /// Runs `task` on its own and sends the message it ends with.
    fn spawn(&self, task: impl Future<Output = Msg> + Send + 'static) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(task.await);
        });
    }

    /// The receiver lives as long as the runner, so the message always arrives.
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
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
    use tokio::sync::mpsc::UnboundedReceiver;
    use v2ex_rs::Error;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::tui::Start;
    use crate::tui::app::testing::{launch, node, tz};

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
    ) -> (Runner, UnboundedReceiver<Msg>) {
        let client = Client::builder().base_url(server.uri()).build().unwrap();
        let (tx, rx) = unbounded_channel();
        let runner = Runner {
            client,
            tx,
            nodes_path,
        };
        (runner, rx)
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

        let Some(Msg::ThreadLoaded { ticket, result, .. }) = rx.recv().await else {
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

        let Some(Msg::ThreadLoaded { result, .. }) = rx.recv().await else {
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
        let Some(Msg::DirectoryLoaded { result, quota }) = rx.recv().await else {
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
        let Some(Msg::DirectoryLoaded { result, .. }) = rx.recv().await else {
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
            let Some(Msg::SaveFailed(reason)) = rx.recv().await else {
                panic!("expected SaveFailed");
            };
            assert!(reason.contains(name), "{name}: {reason}");
        }
        assert!(rx.try_recv().is_err());
    }
}
