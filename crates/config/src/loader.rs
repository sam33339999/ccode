use crate::{error::ConfigError, schema::Config};
use std::path::Path;

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
        assert_eq!(
            or.default_model.as_deref(),
            Some("anthropic/claude-3-5-sonnet")
        );
    }

    #[test]
    fn routing_defaults_to_manual() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.routing.strategy, "manual");
    }

    #[test]
    fn parses_mcp_servers() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
[mcp]
enable_chicago_mcp_feature_gate = true
allow_privileged_computer_use = true

[[mcp.servers]]
name = "filesystem"
command = "node"
args = ["server.js", "--stdio"]
declared_capabilities = ["standard", "privileged_computer_use"]
enable_computer_use = true
"#
        )
        .unwrap();

        let cfg = load_from(f.path()).unwrap();
        assert!(cfg.mcp.enable_chicago_mcp_feature_gate);
        assert!(cfg.mcp.allow_privileged_computer_use);
        assert_eq!(cfg.mcp.servers.len(), 1);
        assert_eq!(cfg.mcp.servers[0].name, "filesystem");
        assert_eq!(cfg.mcp.servers[0].command, "node");
        assert_eq!(
            cfg.mcp.servers[0].args,
            vec!["server.js".to_string(), "--stdio".to_string()]
        );
        assert_eq!(
            cfg.mcp.servers[0].declared_capabilities,
            vec![
                "standard".to_string(),
                "privileged_computer_use".to_string()
            ]
        );
        assert!(cfg.mcp.servers[0].enable_computer_use);
    }

    #[test]
    fn parses_gateway_config() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
[gateway]
port = 7001
workdir = "/tmp/ccode-gateway"

[gateway.telegram]
bot_token = "tg-secret"
webhook_secret = "tg-webhook-secret"

[gateway.discord]
application_public_key = "discord-public-key"
bot_token = "discord-bot-token"
"#
        )
        .unwrap();

        let cfg = load_from(f.path()).unwrap();
        let gateway = cfg.gateway.unwrap();
        assert_eq!(gateway.port, Some(7001));
        assert_eq!(gateway.workdir.as_deref(), Some("/tmp/ccode-gateway"));

        let telegram = gateway.telegram.unwrap();
        assert_eq!(telegram.bot_token, "tg-secret");
        assert_eq!(
            telegram.webhook_secret.as_deref(),
            Some("tg-webhook-secret")
        );

        let discord = gateway.discord.unwrap();
        assert_eq!(discord.application_public_key, "discord-public-key");
        assert_eq!(discord.bot_token.as_deref(), Some("discord-bot-token"));
    }

    #[test]
    fn parses_image_and_provider_vision_config() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
[image]
strategy = "quantize"
max_dimension = 1024

[providers.openrouter]
vision = true
context_window = 200000

[providers.openai]
vision = false
context_window = 128000

[providers.anthropic]
vision = true
context_window = 200000

[providers.zhipu]
vision = true
context_window = 128000

[providers.llamacpp]
vision = false
context_window = 8192
"#
        )
        .unwrap();

        let cfg = load_from(f.path()).unwrap();
        assert_eq!(
            cfg.image.strategy,
            Some(crate::schema::ImageStrategy::Quantize)
        );
        assert_eq!(cfg.image.max_dimension, Some(1024));

        let openrouter = cfg.providers.openrouter.unwrap();
        assert_eq!(openrouter.vision, Some(true));
        assert_eq!(openrouter.context_window, Some(200000));

        let openai = cfg.providers.openai.unwrap();
        assert_eq!(openai.vision, Some(false));
        assert_eq!(openai.context_window, Some(128000));

        let anthropic = cfg.providers.anthropic.unwrap();
        assert_eq!(anthropic.vision, Some(true));
        assert_eq!(anthropic.context_window, Some(200000));

        let zhipu = cfg.providers.zhipu.unwrap();
        assert_eq!(zhipu.vision, Some(true));
        assert_eq!(zhipu.context_window, Some(128000));

        let llamacpp = cfg.providers.llamacpp.unwrap();
        assert_eq!(llamacpp.vision, Some(false));
        assert_eq!(llamacpp.context_window, Some(8192));
    }

    #[test]
    fn image_defaults_are_applied() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(
            cfg.image.strategy,
            Some(crate::schema::ImageStrategy::Resize)
        );
        assert_eq!(cfg.image.max_dimension, Some(2048));
    }
}
