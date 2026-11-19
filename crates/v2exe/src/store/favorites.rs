//! The list of listings the user keeps within reach, saved as `favorites.json`.

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use v2ex_rs::Listing;

use super::paths::write_atomic;

/// The home-page tabs as (name, title), in the order V2EX shows them.
pub const TABS: [(&str, &str); 9] = [
    ("tech", "技术"),
    ("creative", "创意"),
    ("play", "好玩"),
    ("apple", "Apple"),
    ("jobs", "酷工作"),
    ("deals", "交易"),
    ("city", "城市"),
    ("qna", "问与答"),
    ("all", "全部"),
];

/// The display name of the hot and latest feeds and of the known tabs; `None` for a node.
pub fn listing_title(listing: &Listing) -> Option<&'static str> {
    match listing {
        Listing::Hot => Some("最热"),
        Listing::Latest => Some("最新"),
        Listing::Tab(name) => TABS
            .iter()
            .find(|(tab, _)| tab == name)
            .map(|(_, title)| *title),
        Listing::Node(_) => None,
    }
}

fn presets() -> Vec<Favorite> {
    let listing = |tab: &str| Listing::Tab(tab.to_string());
    [
        Listing::Hot,
        Listing::Latest,
        listing("tech"),
        listing("creative"),
        listing("play"),
        listing("qna"),
        listing("all"),
    ]
    .into_iter()
    .map(|listing| Favorite {
        title: listing_title(&listing)
            .expect("every preset has a title")
            .to_string(),
        listing,
    })
    .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Favorite {
    pub listing: Listing,
    pub title: String,
}

pub struct Favorites {
    path: Option<PathBuf>,
    items: Vec<Favorite>,
}

impl Favorites {
    /// A missing or unreadable file gives the seven presets. Without a `path` the list lives
    /// in memory only.
    pub fn load(path: Option<PathBuf>) -> Favorites {
        let items = path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(presets);
        Favorites { path, items }
    }

    pub fn items(&self) -> &[Favorite] {
        &self.items
    }

    pub fn contains(&self, listing: &Listing) -> bool {
        self.items.iter().any(|item| item.listing == *listing)
    }

    /// Returns `false` and changes nothing when the listing is already in the list.
    pub fn add(&mut self, favorite: Favorite) -> bool {
        if self.contains(&favorite.listing) {
            return false;
        }
        self.items.push(favorite);
        true
    }

    /// Does nothing when `index` is out of range.
    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    /// Does nothing when either index is out of range.
    pub fn swap(&mut self, a: usize, b: usize) {
        if a < self.items.len() && b < self.items.len() {
            self.items.swap(a, b);
        }
    }

    /// Without a path there is nothing to write and this is `Ok`.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_atomic(path, &serde_json::to_vec_pretty(&self.items)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_has_the_seven_presets_in_order() {
        let f = Favorites::load(None);
        let titles: Vec<_> = f.items().iter().map(|x| x.title.as_str()).collect();
        assert_eq!(
            titles,
            ["最热", "最新", "技术", "创意", "好玩", "问与答", "全部"]
        );
        assert_eq!(f.items()[0].listing, Listing::Hot);
        assert_eq!(f.items()[1].listing, Listing::Latest);
        assert_eq!(f.items()[2].listing, Listing::Tab("tech".into()));
        assert_eq!(f.items()[6].listing, Listing::Tab("all".into()));
    }

    #[test]
    fn titles_come_from_the_feeds_and_the_tab_list_only() {
        assert_eq!(listing_title(&Listing::Hot), Some("最热"));
        assert_eq!(listing_title(&Listing::Latest), Some("最新"));
        for (name, title) in TABS {
            assert_eq!(listing_title(&Listing::Tab(name.into())), Some(title));
        }
        assert_eq!(listing_title(&Listing::Tab("nope".into())), None);
        assert_eq!(listing_title(&Listing::Node("python".into())), None);
    }

    #[test]
    fn add_remove_swap_and_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state").join("favorites.json");
        let python = Favorite {
            listing: Listing::Node("python".into()),
            title: "Python".into(),
        };

        let mut f = Favorites::load(Some(path.clone()));
        assert!(f.add(python.clone()));
        assert!(f.contains(&python.listing));
        assert!(!f.add(python.clone()));
        assert_eq!(f.items().len(), 8);
        assert_eq!(f.items()[7], python);

        f.swap(0, 7);
        assert_eq!(f.items()[0], python);
        assert_eq!(f.items()[7].listing, Listing::Hot);
        f.remove(1);
        assert_eq!(f.items().len(), 7);
        assert_eq!(f.items()[1].listing, Listing::Tab("tech".into()));
        f.save().unwrap();

        let reloaded = Favorites::load(Some(path));
        assert_eq!(reloaded.items(), f.items());

        let before = f.items().to_vec();
        f.remove(99);
        f.swap(0, 99);
        f.swap(99, 0);
        assert_eq!(f.items(), before);
    }

    #[test]
    fn a_corrupt_or_missing_file_gives_the_presets_and_save_without_a_path_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = Favorites::load(Some(tmp.path().join("missing.json")));
        assert_eq!(missing.items(), Favorites::load(None).items());

        let path = tmp.path().join("favorites.json");
        std::fs::write(&path, "{").unwrap();
        let mut corrupt = Favorites::load(Some(path.clone()));
        assert_eq!(corrupt.items(), Favorites::load(None).items());
        corrupt.add(Favorite {
            listing: Listing::Node("python".into()),
            title: "Python".into(),
        });
        corrupt.save().unwrap();
        assert_eq!(Favorites::load(Some(path)).items().len(), 8);

        Favorites::load(None).save().unwrap();
    }
}
