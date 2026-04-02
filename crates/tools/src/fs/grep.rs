use async_trait::async_trait;
use serde_json::{json, Value};
use ccode_ports::{
    PortError,
    tool::{ToolContext, ToolPort},
};

pub struct FsGrepTool;

#[async_trait]
impl ToolPort for FsGrepTool {
    fn name(&self) -> &str {
        "fs_grep"
    }

    fn description(&self) -> &str {
        "Search for a regex pattern in files. Returns matches with context lines."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File or directory to search" },
                "pattern": { "type": "string", "description": "Regex pattern to search for" },
                "context": { "type": "integer", "description": "Context lines around each match (default 3)" },
                "max_results": { "type": "integer", "description": "Maximum matches to return (default 20)" }
            },
            "required": ["path", "pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing path".into()))?;
        let pattern_str = args["pattern"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing pattern".into()))?;
        let context_lines = args["context"].as_u64().unwrap_or(3) as usize;
        let max_results = args["max_results"].as_u64().unwrap_or(20) as usize;

        let path = std::path::Path::new(path_str);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            ctx.cwd.join(path)
        };

        let regex = regex::Regex::new(pattern_str)
            .map_err(|e| PortError::Tool(format!("invalid regex: {e}")))?;

        let mut all_matches: Vec<Value> = Vec::new();
        let mut total_found: u64 = 0;

        let files: Vec<std::path::PathBuf> = if resolved.is_file() {
            vec![resolved.clone()]
        } else {
            ignore::WalkBuilder::new(&resolved)
                .hidden(true)
                .git_ignore(true)
                .build()
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.path().to_path_buf())
                .collect()
        };

        'outer: for file_path in files {
            let content = match std::fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(_) => continue, // skip binary/unreadable files
            };
            let lines: Vec<&str> = content.lines().collect();

            for (idx, line) in lines.iter().enumerate() {
                if regex.is_match(line) {
                    total_found += 1;
                    if all_matches.len() < max_results {
                        let before_start = idx.saturating_sub(context_lines);
                        let after_end = (idx + context_lines + 1).min(lines.len());

                        let context_before: Vec<String> = lines[before_start..idx]
                            .iter()
                            .map(|l| l.to_string())
                            .collect();
                        let context_after: Vec<String> = lines[idx + 1..after_end]
                            .iter()
                            .map(|l| l.to_string())
                            .collect();

                        all_matches.push(json!({
                            "path": file_path.to_string_lossy(),
                            "line": idx + 1,
                            "content": line,
                            "context_before": context_before,
                            "context_after": context_after
                        }));
                    } else {
                        continue 'outer;
                    }
                }
            }
        }

        let truncated = total_found > max_results as u64;
        let result = json!({
            "matches": all_matches,
            "total_found": total_found,
            "truncated": truncated
        });
        Ok(result.to_string())
    }
}
