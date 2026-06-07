//! Data-directory resolution and config-file management.
//!
//! The config is human-editable TOML stored at
//! `~/.config/life-maintenance/config.toml`. The base directory follows the
//! XDG convention: `$XDG_CONFIG_HOME` when set and non-empty, otherwise
//! `$HOME/.config`.
//!
//! The data directory is resolved with the precedence:
//! `LM_DATA_DIR` env (non-empty) → config file's `data_dir` → error.
//!
//! The decision logic lives in the pure [`resolve`] function so it can be
//! tested without touching the process environment or filesystem; thin IO
//! wrappers read/write the config file and consult the real environment.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where the resolved data dir came from (for `lm config show`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataDirSource {
    /// Resolved from the `LM_DATA_DIR` environment variable.
    Env,
    /// Resolved from the `data_dir` key of the config file at this path.
    ConfigFile(PathBuf),
}

/// On-disk config file shape.
///
/// Deliberately lenient (no `deny_unknown_fields`) so the file can gain keys
/// later without breaking older binaries.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// The configured data directory, if set.
    #[serde(default)]
    pub data_dir: Option<String>,
}

const CONFIG_DIR_NAME: &str = "life-maintenance";
const CONFIG_FILE_NAME: &str = "config.toml";

/// PURE: decide the data dir from already-read inputs.
///
/// Testable without env/fs.
/// - `env` = value of `LM_DATA_DIR` (`None` or empty/blank → ignored).
/// - `configured` = `data_dir` from the config file (empty/blank → ignored).
/// - `config_path` = the config file path, used only to label the source.
///
/// # Errors
/// Returns [`Error::DataFile`] if neither the env nor the config file supplies
/// a non-blank data directory.
pub fn resolve(
    env: Option<&str>,
    configured: Option<&str>,
    config_path: &Path,
) -> Result<(PathBuf, DataDirSource)> {
    if let Some(e) = env {
        if !e.trim().is_empty() {
            return Ok((PathBuf::from(e), DataDirSource::Env));
        }
    }
    if let Some(c) = configured {
        if !c.trim().is_empty() {
            return Ok((
                PathBuf::from(c),
                DataDirSource::ConfigFile(config_path.to_path_buf()),
            ));
        }
    }
    Err(Error::DataFile(
        "data directory not configured: set LM_DATA_DIR or run `lm config set <path>`".to_string(),
    ))
}

/// Resolve the user's home directory via the `HOME` environment variable.
///
/// # Errors
/// Returns [`Error::DataFile`] if `HOME` is unset or empty.
pub fn home_dir() -> Result<PathBuf> {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => Ok(PathBuf::from(h)),
        _ => Err(Error::DataFile(
            "cannot determine home directory: HOME is not set".to_string(),
        )),
    }
}

/// Resolve the XDG config base directory.
///
/// Returns `$XDG_CONFIG_HOME` when it is set and non-empty, otherwise
/// `$HOME/.config`.
///
/// # Errors
/// Returns [`Error::DataFile`] if `XDG_CONFIG_HOME` is unset/empty *and* `HOME`
/// is unset (so the fallback cannot be computed).
pub fn config_base_dir() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg));
        }
    }
    Ok(home_dir()?.join(".config"))
}

/// `<base>/life-maintenance` — takes the XDG config base so tests can inject a
/// tempdir.
#[must_use]
pub fn config_dir_in(base: &Path) -> PathBuf {
    base.join(CONFIG_DIR_NAME)
}

/// `<base>/life-maintenance/config.toml`.
#[must_use]
pub fn config_file_in(base: &Path) -> PathBuf {
    config_dir_in(base).join(CONFIG_FILE_NAME)
}

/// Read and parse the config file at `path`.
///
/// A missing file yields [`Config::default`] (so first-run is not an error).
///
/// # Errors
/// Returns [`Error::Io`] if the file exists but cannot be read, or
/// [`Error::DataFile`] (naming the file) if its contents are not valid TOML.
pub fn read_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    toml::from_str(&text).map_err(|e| Error::DataFile(format!("{}: {e}", path.display())))
}

/// Write `data_dir` into the config file under the XDG config `base`.
///
/// Creates the config directory (`<base>/life-maintenance`) if needed and
/// writes TOML. Returns the config file path that was written.
///
/// # Errors
/// Returns [`Error::Io`] if the config directory cannot be created or the file
/// cannot be written, or [`Error::DataFile`] if serialization fails.
pub fn write_data_dir(base: &Path, data_dir: &Path) -> Result<PathBuf> {
    let dir = config_dir_in(base);
    std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let config = Config {
        data_dir: Some(data_dir.display().to_string()),
    };
    let toml = toml::to_string_pretty(&config)
        .map_err(|e| Error::DataFile(format!("serialize config: {e}")))?;
    let path = config_file_in(base);
    std::fs::write(&path, toml).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

/// Thin convenience: resolve using the real environment.
///
/// Consults `LM_DATA_DIR` and the config file at
/// `~/.config/life-maintenance/config.toml`.
///
/// # Errors
/// Returns [`Error::DataFile`] if the config base cannot be determined, or any
/// error from [`read_config`] / [`resolve`].
pub fn resolve_data_dir() -> Result<(PathBuf, DataDirSource)> {
    let base = config_base_dir()?;
    let cfg_path = config_file_in(&base);
    let configured = read_config(&cfg_path)?.data_dir;
    let env = std::env::var("LM_DATA_DIR").ok();
    resolve(env.as_deref(), configured.as_deref(), &cfg_path)
}

#[cfg(test)]
mod tests {
    // NOTE: `config_base_dir()` and `resolve_data_dir()` read the process
    // environment (HOME / XDG_CONFIG_HOME / LM_DATA_DIR); they are intentionally
    // not unit-tested here to avoid env races between parallel tests.
    use super::{
        config_dir_in, config_file_in, read_config, resolve, write_data_dir, DataDirSource,
    };
    use std::path::{Path, PathBuf};

    fn cfg_path() -> PathBuf {
        PathBuf::from("/home/u/.config/life-maintenance/config.toml")
    }

    #[test]
    fn resolve_prefers_env() {
        let p = cfg_path();
        let (dir, src) = resolve(Some("/x"), Some("/y"), &p).unwrap();
        assert_eq!(dir, PathBuf::from("/x"));
        assert_eq!(src, DataDirSource::Env);
    }

    #[test]
    fn resolve_ignores_empty_env() {
        let p = cfg_path();
        let (dir, src) = resolve(Some(""), Some("/y"), &p).unwrap();
        assert_eq!(dir, PathBuf::from("/y"));
        assert_eq!(src, DataDirSource::ConfigFile(p));
    }

    #[test]
    fn resolve_falls_back_to_config() {
        let p = cfg_path();
        let (dir, src) = resolve(None, Some("/y"), &p).unwrap();
        assert_eq!(dir, PathBuf::from("/y"));
        assert_eq!(src, DataDirSource::ConfigFile(p));
    }

    #[test]
    fn resolve_errors_when_nothing_set() {
        let p = cfg_path();
        assert!(resolve(None, None, &p).is_err());
    }

    #[test]
    fn resolve_ignores_blank_configured() {
        let p = cfg_path();
        assert!(resolve(None, Some("  "), &p).is_err());
    }

    #[test]
    fn read_config_missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.json");
        let config = read_config(&missing).unwrap();
        assert_eq!(config.data_dir, None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let base = tempfile::tempdir().unwrap();
        let returned = write_data_dir(base.path(), Path::new("/log")).unwrap();
        let expected = config_file_in(base.path());
        assert_eq!(returned, expected);
        assert!(
            returned.ends_with("life-maintenance/config.toml"),
            "path was: {}",
            returned.display()
        );

        let written = std::fs::read_to_string(&returned).unwrap();
        assert!(
            written.contains("data_dir"),
            "content was not TOML: {written}"
        );
        assert!(
            !written.trim_start().starts_with('{'),
            "content looked like JSON: {written}"
        );

        let config = read_config(&expected).unwrap();
        assert_eq!(config.data_dir, Some("/log".to_string()));
    }

    #[test]
    fn read_config_malformed_toml_errors_naming_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not = = toml").unwrap();
        let err = read_config(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("config.toml"), "message was: {msg}");
    }

    #[test]
    fn config_paths_have_expected_shape() {
        let base = Path::new("/base");
        assert_eq!(config_dir_in(base), PathBuf::from("/base/life-maintenance"));
        assert_eq!(
            config_file_in(base),
            PathBuf::from("/base/life-maintenance/config.toml")
        );
    }
}
