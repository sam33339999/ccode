use async_trait::async_trait;
use serde_json::{json, Value};
use ccode_ports::{
    PortError,
    tool::{ToolContext, ToolPort},
};

pub struct FsListTool;

#[async_trait]
impl ToolPort for FsListTool {
    fn name(&self) -> &str {
        "fs_list"
    }

    fn description(&self) -> &str {
        "List directory contents as a tree. Respects .gitignore."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory to list (default: cwd)" },
                "depth": { "type": "integer", "description": "Max depth (default 2)" }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let path_str = args["path"].as_str().unwrap_or(".");
        let depth = args["depth"].as_u64().unwrap_or(2) as usize;

        let path = std::path::Path::new(path_str);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else if path_str == "." {
            ctx.cwd.clone()
        } else {
            ctx.cwd.join(path)
        };

        let mut output = format!("{}\n", resolved.display());

        build_tree(&resolved, &resolved, depth, 0, &mut output);

        Ok(output)
    }
}

fn build_tree(
    root: &std::path::Path,
    dir: &std::path::Path,
    max_depth: usize,
    current_depth: usize,
    output: &mut String,
) {
    if current_depth >= max_depth {
        return;
    }

    let walker = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .max_depth(Some(1))
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();

    for entry in walker.flatten() {
        let entry_path = entry.path();
        // Skip the directory itself
        if entry_path == dir {
            continue;
        }

        let indent = "  ".repeat(current_depth + 1);
        let name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");

        if entry_path.is_dir() {
            output.push_str(&format!("{}{}/\n", indent, name));
            if current_depth + 1 < max_depth {
                build_tree(root, entry_path, max_depth, current_depth + 1, output);
            }
        } else {
            output.push_str(&format!("{}{}\n", indent, name));
        }
    }
}
