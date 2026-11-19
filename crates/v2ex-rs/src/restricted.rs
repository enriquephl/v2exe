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

/// Matches ignoring ASCII case, which is the pre-check on a name before any request. The API
/// also resolves other spellings (surrounding whitespace, accented letters) that no comparison
/// here can enumerate, so a node fetched by name is checked again by the name in the response.
pub(crate) fn by_name(name: &str) -> Option<&'static RestrictedNode> {
    RESTRICTED
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
}

/// The restricted node a topic's node reference points at, matching ignoring ASCII case. A
/// reference matches by its name or by its title: the title of a node feed is the node's
/// canonical one, so it catches a name the server resolved from some other spelling, and tab
/// and hot feeds give no other handle.
pub(crate) fn of(node: &NodeRef) -> Option<&'static RestrictedNode> {
    node.name.as_deref().and_then(by_name).or_else(|| {
        RESTRICTED
            .iter()
            .find(|r| r.title.eq_ignore_ascii_case(&node.title))
    })
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
        assert!(of(&node(Some("python"), "Python")).is_none());
    }

    #[test]
    fn matches_by_title_ignoring_case() {
        assert_eq!(of(&node(None, "nsfw")).map(|r| r.name), Some("nsfw"));
        assert_eq!(of(&node(None, "沙盒")).map(|r| r.name), Some("sandbox"));
        assert_eq!(
            of(&node(Some("alias-x"), "NSFW")).map(|r| r.name),
            Some("nsfw")
        );
        assert!(of(&node(None, "Python")).is_none());
    }
}
