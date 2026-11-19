//! The non-interactive flow: settings, client, one subcommand, output. The interactive mode
//! takes its settings and client from [`settings`] too.

use std::ffi::OsStr;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{FixedOffset, Local};
use v2ex_rs::{Client, ClientBuilder, Listing, Node};

use super::args::{Cli, Command};
use super::exit::exit_code;
use super::output::{self, Painter};
use super::topic_ref::parse_topic_ref;
use crate::errors::message;
use crate::render::sanitize;
use crate::store::config::{self, Config, ConfigError, SegmentEdge};
use crate::store::paths::Dirs;

/// What the process environment decides: where the settings file is, whether to color and wrap
/// the text, and the offset times are shown in.
pub struct RunEnv {
    pub config_path: Option<PathBuf>,
    pub color: bool,
    pub width: Option<usize>,
    pub tz: FixedOffset,
}

impl RunEnv {
    /// Colors are on when `NO_COLOR` is unset or empty and stdout is a terminal; the width is
    /// the terminal's, and only known when stdout is a terminal that reports one; times use the
    /// local offset now.
    pub fn from_process() -> RunEnv {
        let tty = io::stdout().is_terminal();
        RunEnv {
            config_path: Dirs::from_process()
                .config
                .map(|dir| dir.join("config.toml")),
            color: color_enabled(tty, std::env::var_os("NO_COLOR").as_deref()),
            width: tty
                .then(|| crossterm::terminal::size().ok())
                .flatten()
                .map(|(columns, _)| usize::from(columns))
                .filter(|&columns| columns > 0),
            tz: *Local::now().offset(),
        }
    }
}

/// Colors need a terminal, and `NO_COLOR` turns them off only when it has a value.
fn color_enabled(tty: bool, no_color: Option<&OsStr>) -> bool {
    tty && no_color.is_none_or(OsStr::is_empty)
}

/// Why a run failed: the process exit code and the text for stderr.
#[derive(Debug)]
pub struct Failure {
    pub code: u8,
    pub message: String,
}

impl Failure {
    pub fn new(code: u8, message: impl Into<String>) -> Failure {
        Failure {
            code,
            message: message.into(),
        }
    }

    /// The error's text can quote the server, so it is sanitized like any other output.
    fn from_error(e: &v2ex_rs::Error, tz: &FixedOffset) -> Failure {
        Failure::new(exit_code(e), sanitize(&message(e, tz)))
    }
}

/// What the settings file, the flags and the process environment decide, for either mode.
pub struct Settings {
    pub client: Client,
    /// Off under `--no-color`, or when the environment turns colors off.
    pub color: bool,
    pub edge: SegmentEdge,
}

/// Reads the settings file and builds the client from it with the flags over its values. A file
/// that cannot be used fails with code 1, and so does a bad `base_url` or `proxy` in it even
/// when a flag overrides that value.
pub fn settings(cli: &Cli, env: &RunEnv) -> Result<Settings, Failure> {
    let config = match &env.config_path {
        Some(path) => config::load(path).map_err(|e| Failure::new(1, e.to_string()))?,
        None => Config::default(),
    };
    if let Some(path) = &env.config_path
        && (config.base_url.is_some() || config.proxy.is_some())
    {
        // A bad value in the file is the file's fault even when a flag overrides it.
        client_builder(config.base_url.clone(), config.proxy.clone(), None)
            .build()
            .map_err(|e| Failure::new(1, config_error(path, &e, &env.tz).to_string()))?;
    }
    let client = client_builder(
        cli.base_url.clone().or(config.base_url),
        cli.proxy.clone().or(config.proxy),
        cli.timeout.or(config.timeout),
    )
    .build()
    .map_err(|e| Failure::from_error(&e, &env.tz))?;
    Ok(Settings {
        client,
        color: env.color && !cli.no_color,
        edge: config.segment_edge.unwrap_or_default(),
    })
}

/// The topic id in a reference given on the command line; anything else is a usage error.
pub fn topic_id(reference: &str) -> Result<u64, Failure> {
    parse_topic_ref(reference)
        .ok_or_else(|| Failure::new(2, format!("无法识别的帖子：{reference}")))
}

/// Runs `command` with the settings of `cli` and writes its output to `out`. A closed `out`
/// ends the run quietly.
pub async fn run(
    command: Command,
    cli: &Cli,
    out: &mut dyn Write,
    env: &RunEnv,
) -> Result<(), Failure> {
    let Settings { client, color, .. } = settings(cli, env)?;
    let session = Session {
        client,
        json: cli.json,
        painter: Painter { color },
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

fn client_builder(
    base_url: Option<String>,
    proxy: Option<String>,
    timeout: Option<u64>,
) -> ClientBuilder {
    let mut builder = Client::builder().user_agent(concat!("v2exe/", env!("CARGO_PKG_VERSION")));
    if let Some(url) = base_url {
        builder = builder.base_url(url);
    }
    if let Some(url) = proxy {
        builder = builder.proxy(url);
    }
    if let Some(secs) = timeout {
        builder = builder.timeout(Duration::from_secs(secs));
    }
    builder
}

/// A library error caused by a value read from the settings file at `path`.
fn config_error(path: &Path, e: &v2ex_rs::Error, tz: &FixedOffset) -> ConfigError {
    ConfigError {
        path: path.to_path_buf(),
        reason: match e {
            v2ex_rs::Error::Config(reason) => reason.clone(),
            other => message(other, tz),
        },
    }
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
        let id = topic_id(reference)?;
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

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;
    use v2ex_rs::Error;

    use super::{Failure, color_enabled};

    #[test]
    fn error_text_from_the_server_loses_its_control_characters() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let error = Error::Decode("bad \x1b[31m\u{9b}2J tag".into());

        let failure = Failure::from_error(&error, &tz);

        assert_eq!(failure.message, "无法解析站点的响应：bad [31m2J tag");
    }

    #[test]
    fn only_a_non_empty_no_color_turns_color_off() {
        assert!(color_enabled(true, None));
        assert!(color_enabled(true, Some("".as_ref())));
        assert!(!color_enabled(true, Some("1".as_ref())));
        assert!(!color_enabled(false, None));
        assert!(!color_enabled(false, Some("".as_ref())));
    }
}
