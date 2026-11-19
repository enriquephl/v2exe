//! Where v2exe keeps its files, and how it writes them.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Each directory already ends in the `v2exe` level; it is `None` when its XDG variable is not
/// an absolute path and `HOME` is not one either.
pub struct Dirs {
    pub config: Option<PathBuf>,
    pub state: Option<PathBuf>,
    pub cache: Option<PathBuf>,
}

impl Dirs {
    pub fn from_env(var: impl Fn(&str) -> Option<OsString>) -> Dirs {
        let absolute = |name: &str| var(name).map(PathBuf::from).filter(|p| p.is_absolute());
        let dir = |xdg: &str, under_home: &str| {
            let base = match absolute(xdg) {
                Some(xdg) => xdg,
                None => absolute("HOME")?.join(under_home),
            };
            Some(base.join("v2exe"))
        };
        Dirs {
            config: dir("XDG_CONFIG_HOME", ".config"),
            state: dir("XDG_STATE_HOME", ".local/state"),
            cache: dir("XDG_CACHE_HOME", ".cache"),
        }
    }

    pub fn from_process() -> Dirs {
        Dirs::from_env(|name| std::env::var_os(name))
    }
}

/// Creates the parent directories, writes a temporary file next to `path`, then renames it
/// to `path`.
///
/// At most one writer per target path per process at a time: the temporary file's name is
/// per process.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temp_name = name.to_os_string();
    temp_name.push(format!(".tmp.{}", std::process::id()));
    let temp = path.with_file_name(temp_name);
    let written = std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_follow_xdg_then_home() {
        let env = |pairs: &'static [(&str, &str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == k)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        let d = Dirs::from_env(env(&[("HOME", "/h")]));
        assert_eq!(d.config, Some("/h/.config/v2exe".into()));
        assert_eq!(d.state, Some("/h/.local/state/v2exe".into()));
        assert_eq!(d.cache, Some("/h/.cache/v2exe".into()));

        let d = Dirs::from_env(env(&[
            ("HOME", "/h"),
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_STATE_HOME", "relative"),
            ("XDG_CACHE_HOME", ""),
        ]));
        assert_eq!(d.config, Some("/c/v2exe".into()));
        assert_eq!(d.state, Some("/h/.local/state/v2exe".into()));
        assert_eq!(d.cache, Some("/h/.cache/v2exe".into()));

        let d = Dirs::from_env(env(&[]));
        assert!(d.config.is_none() && d.state.is_none() && d.cache.is_none());

        let d = Dirs::from_env(env(&[("HOME", ""), ("XDG_CONFIG_HOME", "/c")]));
        assert_eq!(d.config, Some("/c/v2exe".into()));
        assert!(d.state.is_none() && d.cache.is_none());

        let d = Dirs::from_env(env(&[("HOME", "relative"), ("XDG_CONFIG_HOME", "/c")]));
        assert_eq!(d.config, Some("/c/v2exe".into()));
        assert!(d.state.is_none() && d.cache.is_none());
    }

    #[test]
    fn write_atomic_creates_parents_replaces_and_leaves_no_temp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a").join("b");
        let file = dir.join("file");

        write_atomic(&file, b"first").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"first");
        write_atomic(&file, b"second, longer").unwrap();

        assert_eq!(std::fs::read(&file).unwrap(), b"second, longer");
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, [OsString::from("file")]);
    }

    #[test]
    fn write_atomic_reports_an_unwritable_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"a file, not a directory").unwrap();

        let result = write_atomic(&blocker.join("file"), b"x");

        assert!(result.is_err());
        assert_eq!(std::fs::read(&blocker).unwrap(), b"a file, not a directory");
    }
}
