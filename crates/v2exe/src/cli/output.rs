//! Text and JSON output of topics, replies and nodes. Every function returns text ending in a
//! newline.

use chrono::{DateTime, FixedOffset, Utc};
use serde::Serialize;
use v2ex_rs::{Node, Reply, Topic};

use crate::render::{Line, LineKind, Role, Seg, render_html, sanitize, wrap};

/// Applies terminal styles; with `color` off every text comes back unchanged.
#[derive(Debug, Clone, Copy)]
pub struct Painter {
    pub color: bool,
}

#[derive(Debug, Clone, Copy)]
enum Style {
    Dim,
    Bold,
    Green,
    Cyan,
    Blue,
    Magenta,
    BoldYellow,
    BoldRed,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Style::Dim => "2",
            Style::Bold => "1",
            Style::Green => "32",
            Style::Cyan => "36",
            Style::Blue => "34",
            Style::Magenta => "35",
            Style::BoldYellow => "1;33",
            Style::BoldRed => "1;31",
        }
    }
}

impl Painter {
    fn paint(&self, style: Style, text: &str) -> String {
        if self.color && !text.is_empty() {
            format!("\x1b[{}m{text}\x1b[0m", style.code())
        } else {
            text.to_string()
        }
    }

    /// Reply counts stand out the more replies there are.
    fn paint_replies(&self, replies: u32, text: &str) -> String {
        match replies {
            0 => text.to_string(),
            1..=9 => self.paint(Style::Green, text),
            10..=99 => self.paint(Style::BoldYellow, text),
            _ => self.paint(Style::BoldRed, text),
        }
    }

    fn paint_seg(&self, seg: &Seg) -> String {
        match seg.role {
            Role::Bold => self.paint(Style::Bold, &seg.text),
            Role::Link | Role::Mention => self.paint(Style::Blue, &seg.text),
            Role::Plain | Role::Code | Role::Image => seg.text.clone(),
        }
    }
}

/// One row per topic: id, reply count, creation time, author, `[node] title`.
pub fn topic_rows(topics: &[Topic], tz: &FixedOffset, p: &Painter) -> String {
    let mut out = String::new();
    for t in topics {
        let node = t.node.as_ref().map_or_else(String::new, |n| {
            let label = p.paint(Style::Blue, &format!("[{}]", sanitize(&n.title)));
            format!("{label} ")
        });
        out.push_str(&format!(
            "{}  {}  {}  {}  {node}{}\n",
            p.paint(Style::Dim, &format!("{:>7}", t.id)),
            p.paint_replies(t.replies, &format!("{:>4}", t.replies)),
            p.paint(Style::Dim, &time(&t.created, tz, "%m-%d %H:%M")),
            p.paint(
                Style::Cyan,
                &format!("{:<15}", sanitize(&t.author.username))
            ),
            sanitize(&t.title),
        ));
    }
    out
}

/// A topic with its body and, when given, its replies. `width` wraps the text to that many
/// columns, replies included; `None` leaves lines unwrapped.
pub fn topic_detail(
    topic: &Topic,
    replies: Option<&[Reply]>,
    width: Option<usize>,
    tz: &FixedOffset,
    p: &Painter,
) -> String {
    let mut lines = vec![sanitize(&topic.title)];

    let mut meta = vec![p.paint(Style::Cyan, &sanitize(&topic.author.username))];
    if let Some(node) = &topic.node {
        meta.push(p.paint(Style::Blue, &sanitize(&node.title)));
    }
    meta.push(p.paint(Style::Dim, &time(&topic.created, tz, "%Y-%m-%d %H:%M")));
    meta.push(p.paint_replies(topic.replies, &format!("{} 回复", topic.replies)));
    lines.push(meta.join(" · "));
    lines.push(p.paint(Style::Blue, &sanitize(&topic.url)));

    if let Some(html) = &topic.content_html {
        let body = block(&render_html(html), 0, width, p);
        if !body.is_empty() {
            lines.push(String::new());
            lines.extend(body);
        }
    }

    for reply in replies.unwrap_or_default() {
        let author = sanitize(&reply.author.username);
        let owner = if reply.author.username == topic.author.username {
            format!(" {}", p.paint(Style::Magenta, "楼主"))
        } else {
            String::new()
        };
        lines.push(String::new());
        lines.push(format!(
            "{} {}{owner} · {}",
            p.paint(Style::Dim, &format!("#{}", reply.floor)),
            p.paint(Style::Cyan, &author),
            p.paint(Style::Dim, &time(&reply.created, tz, "%m-%d %H:%M")),
        ));
        lines.extend(block(&render_html(&reply.content_html), 4, width, p));
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// One row per node: name, topic count, title.
pub fn node_rows(nodes: &[Node], p: &Painter) -> String {
    let mut out = String::new();
    for n in nodes {
        out.push_str(&format!(
            "{:<20}  {:>6}  {}\n",
            sanitize(&n.name),
            n.topics,
            p.paint(Style::Blue, &sanitize(&n.title)),
        ));
    }
    out
}

pub fn json_topics(topics: &[Topic]) -> String {
    to_json(&topics)
}

/// `{"topic": …, "replies": […]}`; without `replies` the object has the `topic` key only.
pub fn json_topic(topic: &Topic, replies: Option<&[Reply]>) -> String {
    #[derive(Serialize)]
    struct Detail<'a> {
        topic: &'a Topic,
        #[serde(skip_serializing_if = "Option::is_none")]
        replies: Option<&'a [Reply]>,
    }
    to_json(&Detail { topic, replies })
}

pub fn json_nodes(nodes: &[Node]) -> String {
    to_json(&nodes)
}

fn to_json(value: &impl Serialize) -> String {
    let mut out = serde_json::to_string_pretty(value).expect("the public types serialize to JSON");
    out.push('\n');
    out
}

fn time(t: &DateTime<Utc>, tz: &FixedOffset, format: &str) -> String {
    t.with_timezone(tz).format(format).to_string()
}

/// Rendered lines as text lines, each indented by `indent` columns. A code line gets 4 more
/// spaces and a quote line a `> ` prefix. With a `width`, lines are wrapped so that the prefix
/// and indent fit within it.
fn block(lines: &[Line], indent: usize, width: Option<usize>, p: &Painter) -> Vec<String> {
    let pad = " ".repeat(indent);
    let mut out = Vec::new();
    for line in lines {
        let prefix = match line.kind {
            LineKind::Text => "",
            LineKind::Code => "    ",
            LineKind::Quote => "> ",
        };
        let pieces = match width {
            Some(w) => wrap(
                std::slice::from_ref(line),
                w.saturating_sub(indent + prefix.len()),
            ),
            None => vec![line.clone()],
        };
        for piece in &pieces {
            out.push(if piece.segs.is_empty() {
                // Blank lines carry no trailing whitespace; a quote keeps its marker.
                match line.kind {
                    LineKind::Quote => format!("{pad}>"),
                    LineKind::Text | LineKind::Code => String::new(),
                }
            } else {
                let text: String = piece.segs.iter().map(|s| p.paint_seg(s)).collect();
                format!("{pad}{prefix}{text}")
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::display_width;
    use chrono::TimeZone;
    use v2ex_rs::{MemberRef, NodeRef};

    fn tz8() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn plain() -> Painter {
        Painter { color: false }
    }

    fn colored() -> Painter {
        Painter { color: true }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn topic(id: u64, replies: u32, title: &str, html: Option<&str>) -> Topic {
        Topic {
            id,
            title: title.into(),
            url: format!("https://www.v2ex.com/t/{id}"),
            author: MemberRef {
                username: "alice".into(),
                id: Some(7),
            },
            node: Some(NodeRef {
                name: Some("python".into()),
                title: "Python".into(),
            }),
            created: at(1795050000),
            last_touched: None,
            replies,
            last_reply_by: None,
            content: None,
            content_html: html.map(str::to_string),
        }
    }

    fn reply(floor: u32, author: &str, html: &str) -> Reply {
        Reply {
            id: 5000 + u64::from(floor),
            floor,
            author: MemberRef {
                username: author.into(),
                id: None,
            },
            content: String::new(),
            content_html: html.into(),
            created: at(1795055400),
        }
    }

    fn node(name: &str, title: &str, topics: u32) -> Node {
        Node {
            id: 1,
            name: name.into(),
            title: title.into(),
            title_alternative: None,
            topics,
            stars: 0,
            header: None,
            parent_node_name: None,
        }
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn sample_topics() -> Vec<Topic> {
        vec![
            topic(1001, 3, "示例标题一", None),
            topic(1003, 120, "示例标题三", None),
            topic(1005, 15, "示例标题五", None),
            topic(1006, 0, "示例标题六", None),
        ]
    }

    #[test]
    fn rows_have_the_documented_columns() {
        assert_eq!(
            topic_rows(&sample_topics()[..2], &tz8(), &plain()),
            "   1001     3  11-19 09:00  alice            [Python] 示例标题一\n   1003   120  11-19 09:00  alice            [Python] 示例标题三\n"
        );
    }

    #[test]
    fn row_without_node_has_no_label() {
        let mut t = topic(1, 0, "标题", None);
        t.node = None;
        assert!(topic_rows(&[t], &tz8(), &plain()).ends_with("alice            标题\n"));
    }

    #[test]
    fn color_only_adds_escape_codes() {
        let topics = sample_topics();
        let plain_rows = topic_rows(&topics, &tz8(), &plain());
        let colored_rows = topic_rows(&topics, &tz8(), &colored());
        assert!(colored_rows.contains("\x1b["));
        assert_eq!(strip_ansi(&colored_rows), plain_rows);
    }

    #[test]
    fn reply_counts_are_colored_by_size() {
        let rows = topic_rows(&sample_topics(), &tz8(), &colored());
        let lines: Vec<&str> = rows.lines().collect();
        assert!(lines[0].contains("\x1b[32m   3\x1b[0m"));
        assert!(lines[1].contains("\x1b[1;31m 120\x1b[0m"));
        assert!(lines[2].contains("\x1b[1;33m  15\x1b[0m"));
        assert!(lines[3].contains("     0  "));
        for style in ["\x1b[32m", "\x1b[1;33m", "\x1b[1;31m"] {
            assert!(!lines[3].contains(style), "{style:?}");
        }
    }

    #[test]
    fn user_text_loses_control_characters() {
        let mut t = topic(1, 0, "标\x1b[31m题\x07", Some("<p>正文</p>"));
        t.author.username = "a\x1blice".into();
        t.node = Some(NodeRef {
            name: None,
            title: "节\x1b点".into(),
        });
        let rows = topic_rows(std::slice::from_ref(&t), &tz8(), &plain());
        let detail = topic_detail(
            &t,
            Some(&[reply(1, "b\x1bob", "x")]),
            None,
            &tz8(),
            &plain(),
        );
        let nodes = node_rows(&[node("py\x1bthon", "标\x1b题", 1)], &plain());
        for text in [rows, detail, nodes] {
            assert!(!text.contains('\x1b') && !text.contains('\x07'), "{text:?}");
        }
    }

    #[test]
    fn detail_marks_the_owner_and_indents_replies() {
        let t = topic(1001, 2, "标题", Some("<p>第一段</p>"));
        let replies = [
            reply(1, "bob", "<p>@<a href=\"/member/alice\">alice</a> 同意</p>"),
            reply(
                2,
                "alice",
                "<p>谢谢</p><blockquote><p>引用</p></blockquote>",
            ),
        ];
        assert_eq!(
            topic_detail(&t, Some(&replies), None, &tz8(), &plain()),
            "标题\nalice · Python · 2026-11-19 09:00 · 2 回复\nhttps://www.v2ex.com/t/1001\n\n第一段\n\n#1 bob · 11-19 10:30\n    @alice 同意\n\n#2 alice 楼主 · 11-19 10:30\n    谢谢\n\n    > 引用\n"
        );
    }

    #[test]
    fn detail_without_body_node_or_replies_is_three_lines() {
        let mut t = topic(1004, 0, "只有标题", None);
        t.node = None;
        for replies in [None, Some(&[][..])] {
            assert_eq!(
                topic_detail(&t, replies, None, &tz8(), &plain()),
                "只有标题\nalice · 2026-11-19 09:00 · 0 回复\nhttps://www.v2ex.com/t/1004\n"
            );
        }
    }

    #[test]
    fn detail_prefixes_code_and_quote_lines() {
        let t = topic(
            1,
            0,
            "标题",
            Some(
                "<pre><code>fn a() {}\n\nfn b() {}</code></pre><blockquote><p>一</p><p>二</p></blockquote>",
            ),
        );
        assert_eq!(
            topic_detail(&t, None, None, &tz8(), &plain()),
            "标题\nalice · Python · 2026-11-19 09:00 · 0 回复\nhttps://www.v2ex.com/t/1\n\n    fn a() {}\n\n    fn b() {}\n\n> 一\n>\n> 二\n"
        );
    }

    #[test]
    fn detail_color_only_adds_escape_codes() {
        let t = topic(
            1001,
            120,
            "标题",
            Some(
                "<p><strong>粗</strong> <a href=\"https://a.example\">https://a.example</a></p><pre><code>x</code></pre><blockquote>引用</blockquote>",
            ),
        );
        let replies = [
            reply(1, "bob", "@<a href=\"/member/alice\">alice</a> 同意"),
            reply(2, "alice", "第二则"),
        ];
        let plain_text = topic_detail(&t, Some(&replies), Some(40), &tz8(), &plain());
        let colored_text = topic_detail(&t, Some(&replies), Some(40), &tz8(), &colored());
        assert!(colored_text.contains("\x1b[1m粗\x1b[0m"));
        assert!(colored_text.contains("\x1b[35m楼主\x1b[0m"));
        assert!(colored_text.contains("\x1b[34m@alice\x1b[0m"));
        assert_eq!(strip_ansi(&colored_text), plain_text);
    }

    #[test]
    fn width_bounds_every_line() {
        let long = "这是一段很长很长的中文正文，用来检查折行是否把前缀和缩进也算进宽度。";
        let t = topic(
            1,
            1,
            "标题",
            Some(&format!(
                "<p>{long}</p><pre><code>{long}</code></pre><blockquote>{long}</blockquote>"
            )),
        );
        let replies = [reply(
            1,
            "bob",
            &format!("<p>{long}</p><blockquote>{long}</blockquote>"),
        )];
        let text = topic_detail(&t, Some(&replies), Some(24), &tz8(), &plain());
        let body_and_replies = text.lines().skip(3);
        for line in body_and_replies {
            assert!(display_width(line) <= 24, "{line:?}");
        }
        assert!(text.lines().count() > 10);
    }

    #[test]
    fn nodes_have_the_documented_columns() {
        let nodes = [node("python", "Python", 16302), node("qna", "问与答", 5)];
        let rows = node_rows(&nodes, &plain());
        assert_eq!(
            rows,
            "python                 16302  Python\nqna                        5  问与答\n"
        );
        assert_eq!(strip_ansi(&node_rows(&nodes, &colored())), rows);
        assert!(node_rows(&nodes, &colored()).contains("\x1b["));
    }

    #[test]
    fn json_is_pretty_and_ends_with_a_newline() {
        let topics = sample_topics();
        let text = json_topics(&topics[..1]);
        assert!(text.starts_with("[\n  {\n"));
        assert!(text.ends_with("]\n"));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v[0]["created"], "2026-11-19T01:00:00Z");
        assert!(json_nodes(&[node("a", "A", 1)]).ends_with("]\n"));
    }

    #[test]
    fn json_topic_has_replies_only_when_given() {
        let t = topic(1, 0, "标题", None);
        let without: serde_json::Value = serde_json::from_str(&json_topic(&t, None)).unwrap();
        assert!(without.get("replies").is_none());
        let with: serde_json::Value =
            serde_json::from_str(&json_topic(&t, Some(&[reply(1, "bob", "x")]))).unwrap();
        assert_eq!(with["replies"][0]["floor"], 1);
        let empty: serde_json::Value = serde_json::from_str(&json_topic(&t, Some(&[]))).unwrap();
        assert_eq!(empty["replies"], serde_json::json!([]));
    }
}
