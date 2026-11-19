//! Settings file `$XDG_CONFIG_HOME/v2exe/config.toml`; a missing file means defaults.

use std::ffi::OsStr;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Settings read from the file. Unknown keys are ignored.
#[derive(Debug, Default, PartialEq, Deserialize)]
pub struct Config {
    pub base_url: Option<String>,
    pub proxy: Option<String>,
    /// Request timeout in seconds.
    pub timeout: Option<u64>,
}

/// The settings file exists but cannot be used.
#[derive(Debug)]
pub struct ConfigError {
    pub path: PathBuf,
    pub reason: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "配置文件 {} 有误：{}", self.path.display(), self.reason)
    }
}

impl std::error::Error for ConfigError {}

/// `<xdg_config_home>/v2exe/config.toml` when it is set and non-empty, otherwise
/// `<home>/.config/v2exe/config.toml`.
pub fn config_path_from(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|x| !x.is_empty()) {
        Some(xdg) => PathBuf::from(xdg),
        None => Path::new(home?).join(".config"),
    };
    Some(base.join("v2exe").join("config.toml"))
}

/// [`config_path_from`] applied to the process environment.
pub fn config_path() -> Option<PathBuf> {
    config_path_from(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let fail = |reason: String| ConfigError {
        path: path.to_path_buf(),
        reason,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(fail(e.to_string())),
    };
    toml::from_str(&text).map_err(|e| fail(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, content: &str) -> PathBuf {
        let p = dir.path().join("config.toml");
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn missing_file_gives_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            load(&tmp.path().join("nope.toml")).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn reads_all_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(
            &tmp,
            "base_url = \"https://global.v2ex.co\"\nproxy = \"socks5://127.0.0.1:1080\"\ntimeout = 20\n",
        );
        assert_eq!(
            load(&p).unwrap(),
            Config {
                base_url: Some("https://global.v2ex.co".into()),
                proxy: Some("socks5://127.0.0.1:1080".into()),
                timeout: Some(20)
            }
        );
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(&tmp, "color = true\ntimeout = 5\n");
        assert_eq!(
            load(&p).unwrap(),
            Config {
                timeout: Some(5),
                ..Config::default()
            }
        );
    }

    #[test]
    fn syntax_error_names_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(&tmp, "base_url = ");
        let e = load(&p).unwrap_err().to_string();
        assert!(e.contains(&p.display().to_string()) && e.contains("有误"));
    }

    #[test]
    fn wrong_type_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(&tmp, "timeout = \"ten\"");
        assert!(load(&p).is_err());
    }

    #[test]
    fn config_path_rules() {
        assert_eq!(
            config_path_from(Some("/x".as_ref()), Some("/h".as_ref())),
            Some("/x/v2exe/config.toml".into())
        );
        assert_eq!(
            config_path_from(Some("".as_ref()), Some("/h".as_ref())),
            Some("/h/.config/v2exe/config.toml".into())
        );
        assert_eq!(config_path_from(None, None), None);
    }
}
