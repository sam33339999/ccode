use async_trait::async_trait;
use ccode_ports::{
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};

pub struct FsGlobTool;

#[async_trait]
impl ToolPort for FsGlobTool {
    fn name(&self) -> &str {
        "fs_glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern under a root directory."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g. **/*.rs)" },
                "root": { "type": "string", "description": "Root directory to search (default: cwd)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let pattern_str = args["pattern"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing pattern".into()))?;
        let root_str = args["root"].as_str().unwrap_or(".");

        let root = if root_str == "." {
            ctx.cwd.clone()
        } else {
            let p = std::path::Path::new(root_str);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                ctx.cwd.join(p)
            }
        };

        let glob_pattern = glob::Pattern::new(pattern_str)
            .map_err(|e| PortError::Tool(format!("invalid glob pattern: {e}")))?;

        let mut matches: Vec<String> = Vec::new();

        for entry in ignore::WalkBuilder::new(&root)
            .hidden(true)
            .git_ignore(true)
            .build()
            .flatten()
        {
            let entry_path = entry.path();
            if !entry_path.is_file() {
                continue;
            }

            // Match against the path relative to root, or the full path
            let rel = entry_path.strip_prefix(&root).unwrap_or(entry_path);
            let rel_str = rel.to_string_lossy();

            if glob_pattern.matches(&rel_str) || glob_pattern.matches_path(entry_path) {
                matches.push(entry_path.to_string_lossy().into_owned());
            }
        }

        matches.sort();
        let count = matches.len() as u64;
        let result = json!({ "matches": matches, "count": count });
        Ok(result.to_string())
    }
}
