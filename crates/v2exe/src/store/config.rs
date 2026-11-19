//! The settings file `config.toml` in the config directory; a missing file means defaults.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// How the colored segments of the interface end.
#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentEdge {
    #[default]
    Plain,
    Powerline,
}

/// Settings read from the file. Unknown keys are ignored.
#[derive(Debug, Default, PartialEq, Deserialize)]
pub struct Config {
    pub base_url: Option<String>,
    pub proxy: Option<String>,
    /// Request timeout in seconds.
    pub timeout: Option<u64>,
    /// `None` means [`SegmentEdge::Plain`].
    pub segment_edge: Option<SegmentEdge>,
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
    let config: Config = toml::from_str(&text).map_err(|e| fail(e.to_string()))?;
    if config.timeout == Some(0) {
        return Err(fail("timeout 必须大于 0".to_string()));
    }
    Ok(config)
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
            "base_url = \"https://global.v2ex.co\"\nproxy = \"socks5://127.0.0.1:1080\"\ntimeout = 20\nsegment_edge = \"powerline\"\n",
        );
        assert_eq!(
            load(&p).unwrap(),
            Config {
                base_url: Some("https://global.v2ex.co".into()),
                proxy: Some("socks5://127.0.0.1:1080".into()),
                timeout: Some(20),
                segment_edge: Some(SegmentEdge::Powerline),
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
    fn zero_timeout_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(&tmp, "timeout = 0");
        let e = load(&p).unwrap_err().to_string();
        assert!(e.contains(&p.display().to_string()) && e.contains("timeout"));
    }

    #[test]
    fn segment_edge_parses_and_rejects_unknown_values() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(&tmp, "segment_edge = \"powerline\"\n");
        assert_eq!(load(&p).unwrap().segment_edge, Some(SegmentEdge::Powerline));

        let p = write(&tmp, "segment_edge = \"plain\"\n");
        assert_eq!(load(&p).unwrap().segment_edge, Some(SegmentEdge::Plain));

        let p = write(&tmp, "timeout = 5\n");
        assert_eq!(load(&p).unwrap().segment_edge, None);

        let p = write(&tmp, "segment_edge = \"round\"\n");
        let e = load(&p).unwrap_err().to_string();
        assert!(
            e.contains(&p.display().to_string()) && e.contains("有误"),
            "{e}"
        );
    }
}
