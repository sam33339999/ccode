use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    pub sandbox: Option<SandboxConfig>,
    pub memory: Option<MemoryConfig>,
    #[serde(default)]
    pub context: ContextConfig,
}

// ── Context / compression ─────────────────────────────────────────────────────

/// Controls the agentic loop's context window management.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextConfig {
    /// Trigger compression when total context exceeds this many characters
    /// (rough estimate: 4 chars ≈ 1 token). Default: 600_000 (~150k tokens).
    pub compress_chars_threshold: Option<usize>,
    /// Number of most-recent messages to keep verbatim after compression.
    /// Default: 8.
    pub keep_recent_messages: Option<usize>,
    /// Truncate a single tool result that exceeds this many characters.
    /// Default: 40_000.
    pub tool_result_max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxConfig {
    pub cwd: Option<String>,
    /// "any" | "cwd" | "none"
    pub fs_read: Option<String>,
    /// "any" | "cwd" | "none"
    pub fs_write: Option<String>,
    /// "any" | "none" | comma-separated allowlist
    pub shell: Option<String>,
    pub web_fetch: Option<bool>,
    pub browser: Option<bool>,
}

// ── Providers ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    pub openrouter: Option<OpenRouterConfig>,
    pub zhipu: Option<ZhipuConfig>,
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub llamacpp: Option<LlamaCppConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenRouterConfig {
    /// Bearer token. Falls back to `OPENROUTER_API_KEY` env var.
    pub api_key: Option<String>,
    /// Default model slug, e.g. `"anthropic/claude-3-5-sonnet"`.
    pub default_model: Option<String>,
    /// Override base URL (useful for local proxies).
    pub base_url: Option<String>,
    /// Optional HTTP `Referer` header sent to OpenRouter for attribution.
    pub site_url: Option<String>,
    /// Optional `X-Title` header sent to OpenRouter.
    pub site_name: Option<String>,
}

impl OpenRouterConfig {
    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
    }

    pub fn resolved_base_url(&self) -> String {
        self.base_url
            .clone()
            .or_else(|| std::env::var("OPENROUTER_BASE_URL").ok())
            .unwrap_or_else(|| "https://openrouter.ai/api/v1".into())
    }

    pub fn resolved_default_model(&self) -> String {
        self.default_model
            .clone()
            .or_else(|| std::env::var("OPENROUTER_DEFAULT_MODEL").ok())
            .unwrap_or_else(|| "openai/gpt-4o-mini".into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ZhipuConfig {
    /// Bearer token. Falls back to `ZHIPU_API_KEY` env var.
    pub api_key: Option<String>,
    /// Default model slug, e.g. `"glm-4-plus"`.
    pub default_model: Option<String>,
    /// Override base URL (defaults to `https://api.z.ai/api/paas/v4`).
    pub base_url: Option<String>,
    /// Value for the `X-Title` header (required by the coding plan).
    pub title: Option<String>,
}

impl ZhipuConfig {
    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("ZHIPU_API_KEY").ok())
    }

    pub fn resolved_base_url(&self) -> String {
        self.base_url
            .clone()
            .or_else(|| std::env::var("ZHIPU_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.z.ai/api/paas/v4".into())
    }

    pub fn resolved_default_model(&self) -> String {
        self.default_model
            .clone()
            .or_else(|| std::env::var("ZHIPU_DEFAULT_MODEL").ok())
            .unwrap_or_else(|| "glm-4-plus".into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenAiConfig {
    pub api_key: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnthropicConfig {
    /// Bearer token. Falls back to `ANTHROPIC_API_KEY` env var.
    pub api_key: Option<String>,
    /// Default model slug, e.g. `"claude-opus-4-5"`.
    pub default_model: Option<String>,
    /// Override base URL. Defaults to `https://api.anthropic.com/v1`.
    pub base_url: Option<String>,
}

impl AnthropicConfig {
    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    }

    pub fn resolved_base_url(&self) -> String {
        self.base_url
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.anthropic.com/v1".into())
    }

    pub fn resolved_default_model(&self) -> String {
        self.default_model
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_DEFAULT_MODEL").ok())
            .unwrap_or_else(|| "claude-opus-4-5".into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlamaCppConfig {
    /// Optional auth key (some llama.cpp setups enable simple auth).
    /// Falls back to `LLAMACPP_API_KEY`. Defaults to `""`.
    pub api_key: Option<String>,
    /// Falls back to `LLAMACPP_BASE_URL`. Defaults to `http://127.0.0.1:8080/v1`.
    pub base_url: Option<String>,
    /// Falls back to `LLAMACPP_DEFAULT_MODEL`. Defaults to `"default"` (ignored by server).
    pub default_model: Option<String>,
}

impl LlamaCppConfig {
    pub fn resolved_api_key(&self) -> String {
        self.api_key
            .clone()
            .or_else(|| std::env::var("LLAMACPP_API_KEY").ok())
            .unwrap_or_default()
    }

    pub fn resolved_base_url(&self) -> String {
        self.base_url
            .clone()
            .or_else(|| std::env::var("LLAMACPP_BASE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:8080/v1".into())
    }

    pub fn resolved_default_model(&self) -> String {
        self.default_model
            .clone()
            .or_else(|| std::env::var("LLAMACPP_DEFAULT_MODEL").ok())
            .unwrap_or_else(|| "default".into())
    }
}

// ── Routing ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// `manual` | `failover` | `round_robin` | `cost_optimized`
    #[serde(default = "default_strategy")]
    pub strategy: String,
    /// Provider name used when strategy = "manual" or as primary for failover.
    pub default_provider: Option<String>,
}

fn default_strategy() -> String {
    "manual".into()
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            strategy: default_strategy(),
            default_provider: None,
        }
    }
}

// ── Memory ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryConfig {
    /// "fts5"（預設，不需 embedding）| "vector"（需設定 embedding provider）
    #[serde(default = "default_memory_backend")]
    pub backend: String,
    /// SQLite 資料庫路徑。None = ~/.ccode/memory.db
    pub db_path: Option<String>,
    /// Embedding provider 設定（backend = "vector" 時需要）
    pub embedding: Option<EmbeddingConfig>,
}

fn default_memory_backend() -> String { "fts5".into() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingConfig {
    /// "openai" | "llamacpp" | "zhipu"
    pub provider: Option<String>,
    pub openai: Option<EmbeddingOpenAiConfig>,
    pub llamacpp: Option<EmbeddingLlamaCppConfig>,
    pub zhipu: Option<EmbeddingZhipuConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingOpenAiConfig {
    /// env: OPENAI_API_KEY
    pub api_key: Option<String>,
    /// env: OPENAI_EMBEDDING_MODEL，預設 "text-embedding-3-small"
    pub model: Option<String>,
    /// env: OPENAI_BASE_URL，預設 "https://api.openai.com/v1"
    pub base_url: Option<String>,
}

impl EmbeddingOpenAiConfig {
    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key.clone().or_else(|| std::env::var("OPENAI_API_KEY").ok())
    }
    pub fn resolved_model(&self) -> String {
        self.model.clone()
            .or_else(|| std::env::var("OPENAI_EMBEDDING_MODEL").ok())
            .unwrap_or_else(|| "text-embedding-3-small".into())
    }
    pub fn resolved_base_url(&self) -> String {
        self.base_url.clone()
            .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.openai.com/v1".into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingLlamaCppConfig {
    /// env: LLAMACPP_BASE_URL，預設 "http://127.0.0.1:8080/v1"
    pub base_url: Option<String>,
    /// env: LLAMACPP_EMBEDDING_MODEL，預設 "default"
    pub model: Option<String>,
    /// env: LLAMACPP_API_KEY，預設 ""
    pub api_key: Option<String>,
}

impl EmbeddingLlamaCppConfig {
    pub fn resolved_base_url(&self) -> String {
        self.base_url.clone()
            .or_else(|| std::env::var("LLAMACPP_BASE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:8080/v1".into())
    }
    pub fn resolved_model(&self) -> String {
        self.model.clone()
            .or_else(|| std::env::var("LLAMACPP_EMBEDDING_MODEL").ok())
            .unwrap_or_else(|| "default".into())
    }
    pub fn resolved_api_key(&self) -> String {
        self.api_key.clone()
            .or_else(|| std::env::var("LLAMACPP_API_KEY").ok())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingZhipuConfig {
    /// env: ZHIPU_API_KEY
    pub api_key: Option<String>,
    /// env: ZHIPU_EMBEDDING_MODEL，預設 "embedding-3"
    pub model: Option<String>,
    /// env: ZHIPU_BASE_URL
    pub base_url: Option<String>,
}

impl EmbeddingZhipuConfig {
    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key.clone().or_else(|| std::env::var("ZHIPU_API_KEY").ok())
    }
    pub fn resolved_model(&self) -> String {
        self.model.clone()
            .or_else(|| std::env::var("ZHIPU_EMBEDDING_MODEL").ok())
            .unwrap_or_else(|| "embedding-3".into())
    }
    pub fn resolved_base_url(&self) -> String {
        self.base_url.clone()
            .or_else(|| std::env::var("ZHIPU_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.z.ai/api/paas/v4".into())
    }
}
