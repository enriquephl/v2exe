//! Topic references given on the command line: a bare id or a topic link.

/// The topic id in `s`: either the whole string is a positive integer, or the digits
/// right after `/t/` form one (ended by `#`, `?`, `/` or the end of the string).
pub fn parse_topic_ref(s: &str) -> Option<u64> {
    let digits = if s.bytes().all(|b| b.is_ascii_digit()) {
        s
    } else {
        let rest = &s[s.find("/t/")? + 3..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if !matches!(rest[end..].chars().next(), None | Some('#' | '?' | '/')) {
            return None;
        }
        &rest[..end]
    };
    digits.parse().ok().filter(|&id| id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_refs() {
        for (s, want) in [
            ("123", Some(123)),
            ("https://www.v2ex.com/t/123#reply5", Some(123)),
            ("v2ex.com/t/123?p=2", Some(123)),
            ("/t/123", Some(123)),
            ("abc", None),
            ("/t/", None),
            ("0", None),
            ("+5", None),
            ("/t/123abc", None),
            ("/t/99999999999999999999", None),
        ] {
            assert_eq!(parse_topic_ref(s), want, "{s}");
        }
    }
}
