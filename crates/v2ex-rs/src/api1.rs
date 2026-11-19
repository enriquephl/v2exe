//! Parses V2EX API 1.0 JSON responses into the public types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, de::DeserializeOwned};

use crate::{Error, MemberRef, Node, NodeRef, Reply, Result, Topic};

/// Parses an array of topics: hot, latest, `show.json?id=` and `show.json?node_name=`
/// all return this shape.
pub(crate) fn parse_topics(body: &[u8]) -> Result<Vec<Topic>> {
    decode::<Vec<RawTopic>>(body)?
        .into_iter()
        .map(RawTopic::into_topic)
        .collect()
}

/// Parses a topic's replies. Floors count from 1 in array order.
pub(crate) fn parse_replies(body: &[u8]) -> Result<Vec<Reply>> {
    decode::<Vec<RawReply>>(body)?
        .into_iter()
        .zip(1u32..)
        .map(|(raw, floor)| raw.into_reply(floor))
        .collect()
}

/// Parses the object returned by `nodes/show.json`.
pub(crate) fn parse_node(body: &[u8]) -> Result<Node> {
    Ok(decode::<RawNode>(body)?.into_node())
}

/// Parses the array returned by `nodes/all.json`.
pub(crate) fn parse_nodes(body: &[u8]) -> Result<Vec<Node>> {
    Ok(decode::<Vec<RawNode>>(body)?
        .into_iter()
        .map(RawNode::into_node)
        .collect())
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    serde_json::from_slice(body).map_err(|e| Error::Decode(e.to_string()))
}

fn timestamp(secs: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp(secs, 0)
        .ok_or_else(|| Error::Decode(format!("timestamp out of range: {secs}")))
}

/// The API sends `""` where a value is absent.
fn non_empty(text: Option<String>) -> Option<String> {
    text.filter(|t| !t.is_empty())
}

#[derive(Deserialize)]
struct RawMember {
    id: u64,
    username: String,
}

impl From<RawMember> for MemberRef {
    fn from(raw: RawMember) -> Self {
        MemberRef {
            username: raw.username,
            id: Some(raw.id),
        }
    }
}

#[derive(Deserialize)]
struct RawTopic {
    id: u64,
    title: String,
    url: String,
    content: Option<String>,
    content_rendered: Option<String>,
    replies: u32,
    created: i64,
    last_touched: Option<i64>,
    last_reply_by: Option<String>,
    member: RawMember,
    node: RawTopicNode,
}

/// The node embedded in a topic; only the fields `NodeRef` keeps.
#[derive(Deserialize)]
struct RawTopicNode {
    name: String,
    title: String,
}

impl RawTopic {
    fn into_topic(self) -> Result<Topic> {
        Ok(Topic {
            id: self.id,
            title: self.title,
            url: self.url,
            author: self.member.into(),
            node: Some(NodeRef {
                name: Some(self.node.name),
                title: self.node.title,
            }),
            created: timestamp(self.created)?,
            last_touched: self.last_touched.map(timestamp).transpose()?,
            replies: self.replies,
            last_reply_by: non_empty(self.last_reply_by),
            content: non_empty(self.content),
            content_html: non_empty(self.content_rendered),
        })
    }
}

#[derive(Deserialize)]
struct RawReply {
    id: u64,
    content: String,
    content_rendered: String,
    created: i64,
    member: RawMember,
}

impl RawReply {
    fn into_reply(self, floor: u32) -> Result<Reply> {
        Ok(Reply {
            id: self.id,
            floor,
            author: self.member.into(),
            content: self.content,
            content_html: self.content_rendered,
            created: timestamp(self.created)?,
        })
    }
}

#[derive(Deserialize)]
struct RawNode {
    id: u64,
    name: String,
    title: String,
    title_alternative: Option<String>,
    topics: u32,
    stars: u32,
    header: Option<String>,
    parent_node_name: Option<String>,
}

impl RawNode {
    fn into_node(self) -> Node {
        Node {
            id: self.id,
            name: self.name,
            title: self.title,
            title_alternative: non_empty(self.title_alternative),
            topics: self.topics,
            stars: self.stars,
            header: non_empty(self.header),
            parent_node_name: self.parent_node_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const TOPICS: &[u8] = include_bytes!("../tests/fixtures/api1/topics.json");
    const TOPIC: &[u8] = include_bytes!("../tests/fixtures/api1/topic.json");
    const TOPIC_RESTRICTED: &[u8] = include_bytes!("../tests/fixtures/api1/topic_restricted.json");
    const TOPIC_EMPTY: &[u8] = include_bytes!("../tests/fixtures/api1/topic_empty.json");
    const REPLIES: &[u8] = include_bytes!("../tests/fixtures/api1/replies.json");
    const NODE: &[u8] = include_bytes!("../tests/fixtures/api1/node.json");
    const NODES: &[u8] = include_bytes!("../tests/fixtures/api1/nodes.json");

    #[test]
    fn parse_topics_maps_fields() {
        let t = &parse_topics(TOPICS).unwrap()[0];
        assert_eq!((t.id, t.title.as_str(), t.replies), (1001, "示例标题一", 3));
        assert_eq!(
            t.author,
            MemberRef {
                username: "alice".into(),
                id: Some(7)
            }
        );
        assert_eq!(
            t.node,
            Some(NodeRef {
                name: Some("python".into()),
                title: "Python".into()
            })
        );
        assert_eq!(t.created, Utc.timestamp_opt(1795050000, 0).unwrap());
        assert_eq!(
            t.last_touched,
            Some(Utc.timestamp_opt(1795057200, 0).unwrap())
        );
        assert_eq!(t.last_reply_by.as_deref(), Some("bob"));
        assert_eq!(t.content.as_deref(), Some("第一段"));
        assert_eq!(t.content_html.as_deref(), Some("<p>第一段</p>"));
    }

    #[test]
    fn parse_topics_turns_empty_strings_into_none() {
        let t = &parse_topics(TOPICS).unwrap()[1];
        assert_eq!(
            (
                t.last_reply_by.clone(),
                t.content.clone(),
                t.content_html.clone()
            ),
            (None, None, None)
        );
    }

    #[test]
    fn parse_topics_accepts_empty_array() {
        assert!(parse_topics(b"[]").unwrap().is_empty());
    }

    #[test]
    fn parse_topics_rejects_html() {
        assert!(matches!(parse_topics(b"<html>"), Err(Error::Decode(_))));
    }

    #[test]
    fn parse_topics_rejects_missing_required_field() {
        assert!(matches!(
            parse_topics(br#"[{"id": 1, "title": "x"}]"#),
            Err(Error::Decode(_))
        ));
    }

    #[test]
    fn parse_topics_reads_single_topic_fixtures() {
        for (body, id) in [(TOPIC, 1001), (TOPIC_RESTRICTED, 1002), (TOPIC_EMPTY, 1004)] {
            let topics = parse_topics(body).unwrap();
            assert_eq!(topics.iter().map(|t| t.id).collect::<Vec<_>>(), vec![id]);
        }
    }

    #[test]
    fn parse_replies_numbers_floors() {
        let r = parse_replies(REPLIES).unwrap();
        assert_eq!(r.iter().map(|r| r.floor).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(
            r[0].author,
            MemberRef {
                username: "bob".into(),
                id: Some(8)
            }
        );
        assert!(r[0].content_html.contains("href=\"/member/alice\""));
    }

    #[test]
    fn parse_node_and_nodes() {
        let n = parse_node(NODE).unwrap();
        assert_eq!(
            (n.name.as_str(), n.topics, n.header.as_deref()),
            ("python", 16302, Some("这里讨论 Python"))
        );
        let all = parse_nodes(NODES).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            (all[2].header.clone(), all[2].parent_node_name.clone()),
            (None, None)
        );
    }
}
