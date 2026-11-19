//! Nodes that can only be viewed when logged in, and where their recent topic ids come from.

use crate::NodeRef;

/// Where a restricted node's recent topic ids can be fetched without logging in.
pub(crate) enum IdSource {
    /// API 1.0 `topics/show.json?node_name=`.
    Api,
    /// The node's Atom feed.
    Feed,
}

pub(crate) struct RestrictedNode {
    pub name: &'static str,
    pub title: &'static str,
    pub source: IdSource,
}

pub(crate) const RESTRICTED: &[RestrictedNode] = &[
    RestrictedNode {
        name: "flamewar",
        title: "水深火热",
        source: IdSource::Api,
    },
    RestrictedNode {
        name: "nsfw",
        title: "NSFW",
        source: IdSource::Feed,
    },
    RestrictedNode {
        name: "sandbox",
        title: "沙盒",
        source: IdSource::Feed,
    },
];

pub(crate) fn by_name(name: &str) -> Option<&'static RestrictedNode> {
    RESTRICTED.iter().find(|r| r.name == name)
}

/// The restricted node a topic's node reference points at. A reference with a name is matched by
/// name; one with only a title (tab and hot feeds) is matched by title.
pub(crate) fn of(node: &NodeRef) -> Option<&'static RestrictedNode> {
    match node.name.as_deref() {
        Some(name) => by_name(name),
        None => RESTRICTED.iter().find(|r| r.title == node.title),
    }
}
