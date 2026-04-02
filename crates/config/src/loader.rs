use std::path::Path;
use crate::{error::ConfigError, schema::Config};

/// Load config from the default location (`~/.ccode/config.toml`).
/// Missing file is not an error — returns `Config::default()`.
pub fn load() -> Result<Config, ConfigError> {
    load_from(&crate::paths::ccode_dir().join("config.toml"))
}

/// Load config from an explicit path.
pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
    toml::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn missing_file_returns_default() {
        let result = load_from(Path::new("/tmp/does_not_exist_ccode.toml"));
        assert!(result.is_ok());
        let cfg = result.unwrap();
        assert!(cfg.providers.openrouter.is_none());
    }

    #[test]
    fn parses_openrouter_config() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
[providers.openrouter]
api_key = "sk-or-test"
default_model = "anthropic/claude-3-5-sonnet"
"#
        )
        .unwrap();
        let cfg = load_from(f.path()).unwrap();
        let or = cfg.providers.openrouter.unwrap();
        assert_eq!(or.api_key.as_deref(), Some("sk-or-test"));
        assert_eq!(or.default_model.as_deref(), Some("anthropic/claude-3-5-sonnet"));
    }

    #[test]
    fn routing_defaults_to_manual() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.routing.strategy, "manual");
    }
}
