use crate::{PortError, provider::ToolDefinition};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub permission: Permission,
}

#[derive(Debug, Clone)]
pub struct Permission {
    pub fs_read: FsPolicy,
    pub fs_write: FsPolicy,
    pub shell: ShellPolicy,
    pub web_fetch: bool,
    pub browser: bool,
}

#[derive(Debug, Clone)]
pub enum FsPolicy {
    None,
    Cwd,
    Any,
    Paths(Vec<PathBuf>),
}

#[derive(Debug, Clone)]
pub enum ShellPolicy {
    None,
    Any,
    Allowlist(Vec<String>),
}

impl Default for Permission {
    fn default() -> Self {
        Self {
            fs_read: FsPolicy::None,
            fs_write: FsPolicy::None,
            shell: ShellPolicy::None,
            web_fetch: false,
            browser: false,
        }
    }
}

impl ToolContext {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            permission: Permission::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_defaults_to_fail_closed() {
        let permission = Permission::default();

        assert!(matches!(permission.fs_read, FsPolicy::None));
        assert!(matches!(permission.fs_write, FsPolicy::None));
        assert!(matches!(permission.shell, ShellPolicy::None));
        assert!(!permission.web_fetch);
        assert!(!permission.browser);
    }
}

#[async_trait]
pub trait ToolPort: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError>;
}

#[async_trait]
impl<T: ToolPort + ?Sized> ToolPort for Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn description(&self) -> &str {
        (**self).description()
    }
    fn parameters_schema(&self) -> Value {
        (**self).parameters_schema()
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        (**self).execute(args, ctx).await
    }
}
