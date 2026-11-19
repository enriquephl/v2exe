use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which stream of topics to list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Listing {
    Hot,
    Latest,
    /// A home-page tab such as `"tech"`, `"creative"` or `"all"`.
    Tab(String),
    /// A node name such as `"python"`.
    Node(String),
}

/// One page of results. Pages count from 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Topic {
    pub id: u64,
    /// Without the `[node title]` prefix that tab and hot feeds add; that part lives in `node`.
    pub title: String,
    pub url: String,
    pub author: MemberRef,
    /// `None` for tab feeds other than hot.
    pub node: Option<NodeRef>,
    pub created: DateTime<Utc>,
    pub last_touched: Option<DateTime<Utc>>,
    pub replies: u32,
    /// Only the API 1.0 source provides it.
    pub last_reply_by: Option<String>,
    /// Source text. Only the API 1.0 source provides it.
    pub content: Option<String>,
    /// Rendered HTML.
    pub content_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    /// Counts from 1, in the order the replies are returned.
    pub floor: u32,
    pub author: MemberRef,
    pub content: String,
    pub content_html: String,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: u64,
    /// The node's name, the segment used in URLs.
    pub name: String,
    pub title: String,
    pub title_alternative: Option<String>,
    pub topics: u32,
    pub stars: u32,
    pub header: Option<String>,
    pub parent_node_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberRef {
    pub username: String,
    pub id: Option<u64>,
}

/// Tab and hot feeds give a node's title only, without its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRef {
    pub name: Option<String>,
    pub title: String,
}

/// The API quota reported by the most recent API response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimit {
    pub limit: u32,
    pub remaining: u32,
    pub reset: DateTime<Utc>,
}
