//! Parses V2EX Atom feeds into the public types.

use feed_rs::model::Entry;

use crate::{Error, MemberRef, NodeRef, Result, Topic};

/// The feed flavours V2EX serves; they differ in how a topic's node is conveyed.
pub(crate) enum FeedKind {
    /// A node feed (`/feed/<name>.xml`). The feed title is the node's title.
    Node(String),
    /// `/feed/tab/hot.xml`. Every entry title starts with `[node title] `.
    Prefixed,
    /// Any other tab feed. Entries carry no node; brackets in a title are the author's own.
    Plain,
}

/// Parses a feed into topics in feed order. Entries whose link has no `/t/<id>` are skipped.
pub(crate) fn parse_feed(body: &[u8], kind: &FeedKind) -> Result<Vec<Topic>> {
    let feed = feed_rs::parser::parse(body).map_err(|e| Error::Decode(e.to_string()))?;
    let feed_node = match kind {
        FeedKind::Node(name) => Some(NodeRef {
            name: Some(name.clone()),
            title: feed
                .title
                .ok_or_else(|| Error::Decode("feed has no title".into()))?
                .content,
        }),
        FeedKind::Prefixed | FeedKind::Plain => None,
    };
    feed.entries
        .into_iter()
        .filter_map(|entry| into_topic(entry, kind, feed_node.as_ref()).transpose())
        .collect()
}

fn into_topic(entry: Entry, kind: &FeedKind, feed_node: Option<&NodeRef>) -> Result<Option<Topic>> {
    let Some((id, url, replies)) = entry
        .links
        .iter()
        .find_map(|link| parse_topic_link(&link.href))
    else {
        return Ok(None);
    };
    let missing = |what: &str| Error::Decode(format!("feed entry for topic {id} has no {what}"));

    let title = entry.title.ok_or_else(|| missing("title"))?.content;
    let (node, title) = match kind {
        FeedKind::Node(_) => (feed_node.cloned(), title),
        FeedKind::Prefixed => split_node_prefix(&title)
            .map(|(node_title, rest)| {
                let node = NodeRef {
                    name: None,
                    title: node_title.to_owned(),
                };
                (Some(node), rest.to_owned())
            })
            .unwrap_or((None, title)),
        FeedKind::Plain => (None, title),
    };
    let username = entry
        .authors
        .into_iter()
        .find_map(|author| author.name)
        .ok_or_else(|| missing("author"))?;

    Ok(Some(Topic {
        id,
        title,
        url,
        author: MemberRef { username, id: None },
        node,
        created: entry.published.ok_or_else(|| missing("published time"))?,
        last_touched: entry.updated,
        replies,
        last_reply_by: None,
        content: None,
        content_html: entry
            .content
            .and_then(|content| content.body)
            .filter(|body| !body.is_empty()),
    }))
}

/// Reads a link such as `https://www.v2ex.com/t/2001#reply16` into the topic id, the link
/// without its fragment, and the reply count (0 without a `#reply<N>` fragment).
fn parse_topic_link(href: &str) -> Option<(u64, String, u32)> {
    let (url, fragment) = href.split_once('#').unwrap_or((href, ""));
    let after = url.rsplit_once("/t/")?.1;
    let digits_end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    let id = after[..digits_end].parse().ok()?;
    let replies = fragment
        .strip_prefix("reply")
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    Some((id, url.to_owned(), replies))
}

/// Splits `[node title] topic title` at the first `]`, which must be followed by a space.
fn split_node_prefix(title: &str) -> Option<(&str, &str)> {
    let (node_title, rest) = title.strip_prefix('[')?.split_once(']')?;
    Some((node_title, rest.strip_prefix(' ')?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE_XML: &[u8] = include_bytes!("../tests/fixtures/feed/node.xml");
    const TAB_HOT_XML: &[u8] = include_bytes!("../tests/fixtures/feed/tab_hot.xml");
    const TAB_XML: &[u8] = include_bytes!("../tests/fixtures/feed/tab.xml");

    #[test]
    fn node_feed_maps_entries() {
        let t = parse_feed(NODE_XML, &FeedKind::Node("python".into())).unwrap();
        assert_eq!(
            t.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![2001, 2002, 2003]
        );
        assert_eq!(
            t.iter().map(|t| t.replies).collect::<Vec<_>>(),
            vec![16, 0, 0]
        );
        assert_eq!(t[0].url, "https://www.v2ex.com/t/2001");
        assert_eq!(
            t[0].node,
            Some(NodeRef {
                name: Some("python".into()),
                title: "Python".into()
            })
        );
        assert_eq!(t[0].created.to_rfc3339(), "2026-11-19T01:00:00+00:00");
        assert_eq!(
            t[0].last_touched.unwrap().to_rfc3339(),
            "2026-11-19T03:00:00+00:00"
        );
        assert_eq!(
            t[0].author,
            MemberRef {
                username: "alice".into(),
                id: None
            }
        );
        assert_eq!(
            (t[0].content.clone(), t[0].content_html.as_deref()),
            (None, Some("<p>正文</p>"))
        );
    }

    #[test]
    fn prefixed_feed_splits_node_title() {
        let t = parse_feed(TAB_HOT_XML, &FeedKind::Prefixed).unwrap();
        assert_eq!(
            t.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            vec!["标题甲", "[求助] 标题乙", "标题丙"]
        );
        assert_eq!(
            t.iter()
                .map(|t| t.node.clone().unwrap().title)
                .collect::<Vec<_>>(),
            vec!["Python", "🤖Vibe Coding", "水深火热"]
        );
        assert!(t.iter().all(|t| t.node.as_ref().unwrap().name.is_none()));
    }

    #[test]
    fn plain_feed_keeps_author_brackets() {
        let t = parse_feed(TAB_XML, &FeedKind::Plain).unwrap();
        assert_eq!(t[0].title, "[拼团] 作者自己的方括号");
        assert!(t.iter().all(|t| t.node.is_none()));
    }

    #[test]
    fn parse_feed_rejects_json() {
        assert!(matches!(
            parse_feed(b"[]", &FeedKind::Plain),
            Err(Error::Decode(_))
        ));
    }

    fn entry(title: &str, href: &str, content: &str) -> String {
        format!(
            r#"<entry><title>{title}</title>
            <link rel="alternate" type="text/html" href="{href}" />
            <id>tag:www.v2ex.com,2026-11-19:/x</id>
            <published>2026-11-19T01:00:00Z</published>
            <updated>2026-11-19T03:00:00Z</updated>
            <author><name>alice</name></author>
            <content type="html"><![CDATA[{content}]]></content></entry>"#
        )
    }

    fn feed(entries: &[String]) -> Vec<u8> {
        format!(
            r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>T</title>
            <id>x</id><updated>2026-11-19T03:00:00Z</updated>{}</feed>"#,
            entries.concat()
        )
        .into_bytes()
    }

    #[test]
    fn parse_feed_skips_entries_without_topic_link() {
        let body = feed(&[
            entry("跳过", "https://www.v2ex.com/member/alice", "x"),
            entry("保留", "https://www.v2ex.com/t/7#reply2", "x"),
        ]);
        let t = parse_feed(&body, &FeedKind::Plain).unwrap();
        assert_eq!(t.iter().map(|t| t.id).collect::<Vec<_>>(), vec![7]);
    }

    #[test]
    fn prefixed_feed_needs_a_space_after_the_first_bracket() {
        let body = feed(&[
            entry("没有前缀", "https://www.v2ex.com/t/1", "x"),
            entry("[甲]乙] 丙", "https://www.v2ex.com/t/2", "x"),
        ]);
        let t = parse_feed(&body, &FeedKind::Prefixed).unwrap();
        assert_eq!(t[0].title, "没有前缀");
        assert_eq!(t[1].title, "[甲]乙] 丙");
        assert!(t.iter().all(|t| t.node.is_none()));
    }

    #[test]
    fn empty_content_becomes_none() {
        let body = feed(&[entry("标题", "https://www.v2ex.com/t/1", "")]);
        let t = parse_feed(&body, &FeedKind::Plain).unwrap();
        assert_eq!(t[0].content_html, None);
    }

    #[test]
    fn entry_missing_published_is_a_decode_error() {
        let body = feed(&[entry("标题", "https://www.v2ex.com/t/1", "x")
            .replace("<published>2026-11-19T01:00:00Z</published>", "")]);
        assert!(matches!(
            parse_feed(&body, &FeedKind::Plain),
            Err(Error::Decode(_))
        ));
    }
}
