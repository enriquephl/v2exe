use std::io::{self, IsTerminal};

use clap::Parser;
use v2ex_rs::Listing;
use v2exe::cli::args::{Cli, Command};
use v2exe::cli::run::{Failure, RunEnv, run, settings, topic_id};
use v2exe::store::paths::Dirs;
use v2exe::tui::{self, Options, Start};

fn main() {
    let mut cli = Cli::parse();
    let env = RunEnv::from_process();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the tokio runtime starts");
    let result = match cli.command.take() {
        Some(command) if !cli.interactive => {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            runtime.block_on(run(command, &cli, &mut out, &env))
        }
        command => runtime.block_on(interactive(command, &cli, &env)),
    };
    if let Err(failure) = result {
        eprintln!("v2exe: {}", failure.message);
        std::process::exit(i32::from(failure.code));
    }
}

/// Opens the interface where `command` points, or on the start screen without one. Usage and
/// settings errors are reported before the terminal is needed.
async fn interactive(command: Option<Command>, cli: &Cli, env: &RunEnv) -> Result<(), Failure> {
    if cli.json {
        return Err(Failure::new(2, "--json 只用于非互动式子命令"));
    }
    let settings = settings(cli, env)?;
    let start = start(command)?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Failure::new(
            2,
            "互动式界面需要在终端里运行；非互动式用法见 v2exe --help",
        ));
    }
    let options = Options {
        start,
        edge: settings.edge,
        color: settings.color,
        tz: env.tz,
        dirs: Dirs::from_process(),
    };
    tui::run(settings.client, options)
        .await
        .map_err(|e| Failure::new(1, format!("终端出错：{e}")))
}

/// The screen the interface opens on: the listing or the topic `command` names; the start
/// screen for `nodes` and without a command.
fn start(command: Option<Command>) -> Result<Start, Failure> {
    Ok(match command {
        None | Some(Command::Nodes { .. }) => Start::Home,
        Some(Command::Hot { .. }) => Start::Listing(Listing::Hot),
        Some(Command::Latest { .. }) => Start::Listing(Listing::Latest),
        Some(Command::Tab { name, .. }) => Start::Listing(Listing::Tab(name)),
        Some(Command::Node { name, .. }) => Start::Listing(Listing::Node(name)),
        Some(Command::Topic { reference, .. }) => Start::Topic(topic_id(&reference)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_of(args: &[&str]) -> Result<Start, Failure> {
        let cli = Cli::try_parse_from(args).unwrap();
        start(cli.command)
    }

    #[test]
    fn each_subcommand_opens_its_own_screen() {
        let cases = [
            (&["v2exe"][..], Start::Home),
            (&["v2exe", "-i", "nodes", "py"], Start::Home),
            (&["v2exe", "-i", "hot"], Start::Listing(Listing::Hot)),
            (&["v2exe", "-i", "latest"], Start::Listing(Listing::Latest)),
            (
                &["v2exe", "-i", "tab", "tech"],
                Start::Listing(Listing::Tab("tech".into())),
            ),
            (
                &["v2exe", "-i", "node", "python"],
                Start::Listing(Listing::Node("python".into())),
            ),
            (
                &["v2exe", "-i", "topic", "https://www.v2ex.com/t/123#reply5"],
                Start::Topic(123),
            ),
        ];
        for (args, want) in cases {
            assert_eq!(start_of(args).unwrap(), want, "{args:?}");
        }

        let failure = start_of(&["v2exe", "-i", "topic", "abc"]).unwrap_err();
        assert_eq!(failure.code, 2);
        assert_eq!(failure.message, "无法识别的帖子：abc");
    }
}
