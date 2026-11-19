//! Command-line definition.

use clap::{Parser, Subcommand};

/// V2EX 终端阅读器
#[derive(Debug, Parser)]
#[command(version, arg_required_else_help = true)]
pub struct Cli {
    /// 输出 JSON
    #[arg(long, global = true)]
    pub json: bool,
    /// 关闭颜色
    #[arg(long, global = true)]
    pub no_color: bool,
    /// 站点地址
    #[arg(long, global = true)]
    pub base_url: Option<String>,
    /// 代理地址
    #[arg(long, global = true)]
    pub proxy: Option<String>,
    /// 请求超时，单位秒
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout: Option<u64>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// 今日热议主题
    Hot {
        /// 最多显示几条
        #[arg(long)]
        limit: Option<usize>,
    },
    /// 最新主题
    Latest {
        /// 最多显示几条
        #[arg(long)]
        limit: Option<usize>,
    },
    /// 某个标签页下的主题
    Tab {
        /// 标签页名称
        name: String,
        /// 最多显示几条
        #[arg(long)]
        limit: Option<usize>,
    },
    /// 某个节点下的主题
    Node {
        /// 节点名称
        name: String,
        /// 最多显示几条
        #[arg(long)]
        limit: Option<usize>,
    },
    /// 查看一个帖子及其回复
    Topic {
        /// 帖子编号或链接
        reference: String,
        /// 不显示回复
        #[arg(long)]
        no_replies: bool,
    },
    /// 列出或搜索节点
    Nodes {
        /// 按名称或标题过滤
        keyword: Option<String>,
        /// 最多显示几条
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, error::ErrorKind};

    #[test]
    fn parses_topic_flags() {
        let c = Cli::try_parse_from(["v2exe", "topic", "123", "--no-replies", "--json"]).unwrap();
        assert!(
            c.json
                && matches!(c.command, Some(Command::Topic { ref reference, no_replies: true }) if reference == "123")
        );
    }

    #[test]
    fn global_flags_follow_the_subcommand() {
        let c = Cli::try_parse_from([
            "v2exe",
            "hot",
            "--limit",
            "5",
            "--json",
            "--no-color",
            "--base-url",
            "https://x.test",
            "--proxy",
            "socks5://p:1",
            "--timeout",
            "7",
        ])
        .unwrap();
        assert!(c.json && c.no_color);
        assert_eq!(c.base_url.as_deref(), Some("https://x.test"));
        assert_eq!(c.proxy.as_deref(), Some("socks5://p:1"));
        assert_eq!(c.timeout, Some(7));
        assert!(matches!(c.command, Some(Command::Hot { limit: Some(5) })));
    }

    #[test]
    fn timeout_must_be_at_least_one_second() {
        for value in ["0", "x"] {
            let err = Cli::try_parse_from(["v2exe", "hot", "--timeout", value]).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ValueValidation, "{value}");
        }
        let c = Cli::try_parse_from(["v2exe", "hot", "--timeout", "1"]).unwrap();
        assert_eq!(c.timeout, Some(1));
    }

    #[test]
    fn version_flag_is_known() {
        assert_eq!(
            Cli::try_parse_from(["v2exe", "--version"])
                .unwrap_err()
                .kind(),
            ErrorKind::DisplayVersion
        );
    }

    #[test]
    fn bare_invocation_shows_help() {
        assert_eq!(
            Cli::try_parse_from(["v2exe"]).unwrap_err().kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
