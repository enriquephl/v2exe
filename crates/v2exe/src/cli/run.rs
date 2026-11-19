//! The non-interactive flow: settings, client, one subcommand, output.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

use chrono::{FixedOffset, Local};
use clap::CommandFactory;
use v2ex_rs::{Client, Listing, Node};

use super::args::{Cli, Command};
use super::exit::{exit_code, message};
use super::output::{self, Painter};
use super::topic_ref::parse_topic_ref;
use crate::config::{self, Config};

/// What the process environment decides: where the settings file is, whether to color and wrap
/// the text, and the offset times are shown in.
pub struct RunEnv {
    pub config_path: Option<PathBuf>,
    pub color: bool,
    pub width: Option<usize>,
    pub tz: FixedOffset,
}

impl RunEnv {
    /// Colors are on when `NO_COLOR` is unset and stdout is a terminal; the width is the
    /// terminal's, and only known when stdout is a terminal that reports one; times use the
    /// local offset now.
    pub fn from_process() -> RunEnv {
        let tty = io::stdout().is_terminal();
        RunEnv {
            config_path: config::config_path(),
            color: tty && std::env::var_os("NO_COLOR").is_none(),
            width: tty
                .then(|| crossterm::terminal::size().ok())
                .flatten()
                .map(|(columns, _)| usize::from(columns))
                .filter(|&columns| columns > 0),
            tz: *Local::now().offset(),
        }
    }
}

/// Why a run failed: the process exit code and the text for stderr.
#[derive(Debug)]
pub struct Failure {
    pub code: u8,
    pub message: String,
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Failure {
        Failure {
            code,
            message: message.into(),
        }
    }

    fn from_error(e: &v2ex_rs::Error, tz: &FixedOffset) -> Failure {
        Failure::new(exit_code(e), message(e, tz))
    }
}

/// Runs the subcommand and writes its output to `out`. A closed `out` ends the run quietly.
pub async fn run(cli: Cli, out: &mut dyn Write, env: &RunEnv) -> Result<(), Failure> {
    let Cli {
        json,
        no_color,
        base_url,
        proxy,
        timeout,
        command,
    } = cli;
    let Some(command) = command else {
        let help = Cli::command().render_help();
        return Err(Failure::new(2, format!("缺少子命令\n\n{help}")));
    };

    let config = match &env.config_path {
        Some(path) => config::load(path).map_err(|e| Failure::new(1, e.to_string()))?,
        None => Config::default(),
    };
    let mut builder = Client::builder().user_agent(concat!("v2exe/", env!("CARGO_PKG_VERSION")));
    if let Some(url) = base_url.or(config.base_url) {
        builder = builder.base_url(url);
    }
    if let Some(url) = proxy.or(config.proxy) {
        builder = builder.proxy(url);
    }
    if let Some(secs) = timeout.or(config.timeout) {
        builder = builder.timeout(Duration::from_secs(secs));
    }
    let session = Session {
        client: builder
            .build()
            .map_err(|e| Failure::from_error(&e, &env.tz))?,
        json,
        painter: Painter {
            color: env.color && !no_color,
        },
        env,
    };

    let text = match command {
        Command::Hot { limit } => session.listing(Listing::Hot, limit).await?,
        Command::Latest { limit } => session.listing(Listing::Latest, limit).await?,
        Command::Tab { name, limit } => session.listing(Listing::Tab(name), limit).await?,
        Command::Node { name, limit } => session.listing(Listing::Node(name), limit).await?,
        Command::Topic {
            reference,
            no_replies,
        } => session.topic(&reference, no_replies).await?,
        Command::Nodes { keyword, limit } => session.nodes(keyword.as_deref(), limit).await?,
    };
    write_out(out, &text)
}

struct Session<'a> {
    client: Client,
    json: bool,
    painter: Painter,
    env: &'a RunEnv,
}

impl Session<'_> {
    async fn listing(&self, listing: Listing, limit: Option<usize>) -> Result<String, Failure> {
        let mut topics = self
            .client
            .topics(&listing, 1)
            .await
            .map_err(|e| self.failure(&e))?
            .items;
        if let Some(n) = limit {
            topics.truncate(n);
        }
        Ok(if self.json {
            output::json_topics(&topics)
        } else {
            output::topic_rows(&topics, &self.env.tz, &self.painter)
        })
    }

    async fn topic(&self, reference: &str, no_replies: bool) -> Result<String, Failure> {
        let id = parse_topic_ref(reference)
            .ok_or_else(|| Failure::new(2, format!("无法识别的帖子：{reference}")))?;
        let topic = self.client.topic(id).await.map_err(|e| self.failure(&e))?;
        let replies = if no_replies {
            None
        } else {
            Some(
                self.client
                    .replies(id, 1)
                    .await
                    .map_err(|e| self.failure(&e))?
                    .items,
            )
        };
        Ok(if self.json {
            output::json_topic(&topic, replies.as_deref())
        } else {
            output::topic_detail(
                &topic,
                replies.as_deref(),
                self.env.width,
                &self.env.tz,
                &self.painter,
            )
        })
    }

    async fn nodes(&self, keyword: Option<&str>, limit: Option<usize>) -> Result<String, Failure> {
        let mut nodes = self.client.nodes().await.map_err(|e| self.failure(&e))?;
        if let Some(keyword) = keyword {
            let keyword = keyword.to_lowercase();
            nodes.retain(|n| mentions(n, &keyword));
        }
        if let Some(n) = limit {
            nodes.truncate(n);
        }
        Ok(if self.json {
            output::json_nodes(&nodes)
        } else {
            output::node_rows(&nodes, &self.painter)
        })
    }

    fn failure(&self, e: &v2ex_rs::Error) -> Failure {
        Failure::from_error(e, &self.env.tz)
    }
}

/// Whether the lowercase `keyword` is part of the node's name, title or alternative title,
/// ignoring case.
fn mentions(node: &Node, keyword: &str) -> bool {
    [
        Some(node.name.as_str()),
        Some(node.title.as_str()),
        node.title_alternative.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|s| s.to_lowercase().contains(keyword))
}

fn write_out(out: &mut dyn Write, text: &str) -> Result<(), Failure> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => {
            Err(Failure::new(1, format!("无法写出结果：{e}")))
        }
        _ => Ok(()),
    }
}
