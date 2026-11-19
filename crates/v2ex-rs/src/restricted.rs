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

/// Matches ignoring ASCII case: the API resolves node names case-insensitively.
pub(crate) fn by_name(name: &str) -> Option<&'static RestrictedNode> {
    RESTRICTED
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
}

/// The restricted node a topic's node reference points at, matching ignoring ASCII case. A
/// reference with a name is matched by name; one with only a title (tab and hot feeds) is
/// matched by title.
pub(crate) fn of(node: &NodeRef) -> Option<&'static RestrictedNode> {
    match node.name.as_deref() {
        Some(name) => by_name(name),
        None => RESTRICTED
            .iter()
            .find(|r| r.title.eq_ignore_ascii_case(&node.title)),
    }
}

#[cfg(test)]
mod tests {
    use super::of;
    use crate::NodeRef;

    fn node(name: Option<&str>, title: &str) -> NodeRef {
        NodeRef {
            name: name.map(str::to_string),
            title: title.to_string(),
        }
    }

    #[test]
    fn matches_by_name_ignoring_case() {
        let found = of(&node(Some("FlameWar"), "x"));
        assert_eq!(found.map(|r| r.name), Some("flamewar"));
        assert!(of(&node(Some("python"), "NSFW")).is_none());
    }

    #[test]
    fn matches_by_title_when_there_is_no_name() {
        assert_eq!(of(&node(None, "nsfw")).map(|r| r.name), Some("nsfw"));
        assert_eq!(of(&node(None, "沙盒")).map(|r| r.name), Some("sandbox"));
        assert!(of(&node(None, "Python")).is_none());
    }
}
