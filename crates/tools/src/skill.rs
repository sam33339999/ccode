use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use ccode_ports::tool::{ToolContext, ToolPort};
use ccode_ports::PortError;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_SCAN_DEPTH: usize = 4;
const MAX_SCAN_DIRS: usize = 2000;

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub skill_dir: PathBuf,
    pub skill_md: PathBuf,
}

/// Discover skills from standard locations, in priority order.
/// Project-level skills (cwd-relative) override user-level skills.
pub fn discover_skills(cwd: &Path) -> Vec<SkillEntry> {
    let home = home_dir();
    let scan_paths = [
        cwd.join(".ccode").join("skills"),
        cwd.join(".agents").join("skills"),
        home.join(".ccode").join("skills"),
        home.join(".agents").join("skills"),
    ];

    let mut by_name: HashMap<String, SkillEntry> = HashMap::new();

    for base in &scan_paths {
        if !base.exists() {
            continue;
        }
        for entry in scan_skill_dir(base) {
            if by_name.contains_key(&entry.name) {
                tracing::warn!(
                    name = %entry.name,
                    path = %entry.skill_md.display(),
                    "skill shadowed by a higher-priority entry"
                );
            } else {
                by_name.insert(entry.name.clone(), entry);
            }
        }
    }

    let mut skills: Vec<SkillEntry> = by_name.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Build the skill catalog for injection into the system prompt.
/// Returns `None` if no skills were found.
pub fn build_skill_catalog(skills: &[SkillEntry]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }

    let mut catalog = String::from(
        "The following skills provide specialized instructions for specific tasks.\n\
         When a task matches a skill's description, call the activate_skill tool\n\
         with the skill's name to load its full instructions.\n\n\
         <available_skills>\n",
    );
    for skill in skills {
        catalog.push_str(&format!(
            "  <skill>\n    <name>{}</name>\n    <description>{}</description>\n  </skill>\n",
            escape_xml(&skill.name),
            escape_xml(&skill.description),
        ));
    }
    catalog.push_str("</available_skills>");
    Some(catalog)
}

/// Augment an optional system prompt with the skill catalog.
pub fn augment_with_skill_catalog(
    persona: Option<String>,
    catalog: &Option<String>,
) -> Option<String> {
    let Some(cat) = catalog.as_deref() else {
        return persona;
    };
    Some(match persona {
        Some(p) => format!("{p}\n\n{cat}"),
        None => cat.to_string(),
    })
}

/// Load a skill's body content for user-explicit activation.
/// Returns `None` if skill is not found.
pub fn load_skill_body(name: &str, skills: &[SkillEntry]) -> Option<String> {
    let skill = skills.iter().find(|s| s.name == name)?;
    let content = std::fs::read_to_string(&skill.skill_md).ok()?;
    let (_, body) = extract_frontmatter(&content).unwrap_or(("", content.as_str()));
    Some(format!(
        "<skill_content name=\"{name}\">\n{body}\n\nSkill directory: {dir}\nRelative paths in this skill are relative to the skill directory.</skill_content>",
        dir = skill.skill_dir.display(),
    ))
}

// ── Internals ─────────────────────────────────────────────────────────────────

fn scan_skill_dir(base: &Path) -> Vec<SkillEntry> {
    let mut results = Vec::new();
    scan_recursive(base, 0, &mut 0, &mut results);
    results
}

fn scan_recursive(dir: &Path, depth: usize, dir_count: &mut usize, out: &mut Vec<SkillEntry>) {
    if depth > MAX_SCAN_DEPTH || *dir_count >= MAX_SCAN_DIRS {
        return;
    }
    *dir_count += 1;

    let skill_md = dir.join("SKILL.md");
    if skill_md.is_file() {
        match parse_skill_md(&skill_md) {
            Ok(entry) => out.push(entry),
            Err(e) => tracing::warn!(
                path = %skill_md.display(),
                "skipping SKILL.md: {e}"
            ),
        }
        // Don't recurse inside a skill directory
        return;
    }

    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if matches!(name.as_ref(), ".git" | "node_modules" | "target" | ".tox" | ".venv") {
            continue;
        }
        scan_recursive(&path, depth + 1, dir_count, out);
    }
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: Option<String>,
}

fn parse_skill_md(path: &Path) -> anyhow::Result<SkillEntry> {
    let content = std::fs::read_to_string(path)?;
    let (yaml, _body) = extract_frontmatter(&content)
        .ok_or_else(|| anyhow::anyhow!("missing YAML frontmatter (--- delimiters)"))?;

    let fm: SkillFrontmatter = serde_yaml::from_str(yaml)
        .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;

    let description = fm
        .description
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing or empty 'description' field"))?;

    let skill_dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent directory"))?
        .to_path_buf();

    Ok(SkillEntry {
        name: fm.name,
        description,
        skill_dir,
        skill_md: path.to_path_buf(),
    })
}

/// Extract `(yaml_frontmatter, body)` from SKILL.md content.
fn extract_frontmatter(content: &str) -> Option<(&str, &str)> {
    let s = content.trim_start();
    let rest = s.strip_prefix("---")?;
    let rest = rest.strip_prefix('\n').or_else(|| rest.strip_prefix("\r\n"))?;
    let close = rest.find("\n---")?;
    let yaml = &rest[..close];
    let after = &rest[close + 4..]; // skip \n---
    let body = after.trim_start_matches(['\n', '\r']);
    Some((yaml, body))
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

// ── ActivateSkillTool ─────────────────────────────────────────────────────────

pub struct ActivateSkillTool {
    skills: Vec<SkillEntry>,
    activated: Mutex<HashSet<String>>,
}

impl ActivateSkillTool {
    pub fn new(skills: Vec<SkillEntry>) -> Self {
        Self {
            skills,
            activated: Mutex::new(HashSet::new()),
        }
    }
}

#[async_trait]
impl ToolPort for ActivateSkillTool {
    fn name(&self) -> &str {
        "activate_skill"
    }

    fn description(&self) -> &str {
        "Activate an Agent Skill by name to load its specialized instructions into context. \
         Use this when a task matches a skill's description in the available_skills catalog."
    }

    fn parameters_schema(&self) -> Value {
        let names: Vec<&str> = self.skills.iter().map(|s| s.name.as_str()).collect();
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the skill to activate.",
                    "enum": names
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String, PortError> {
        let name = args["name"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing: name".into()))?;

        {
            let mut activated = self.activated.lock().unwrap();
            if activated.contains(name) {
                return Ok(format!("[skill '{}' already loaded in this session]", name));
            }
            activated.insert(name.to_string());
        }

        let skill = self
            .skills
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| PortError::Tool(format!("skill not found: {name}")))?;

        let content = std::fs::read_to_string(&skill.skill_md)
            .map_err(|e| PortError::Tool(format!("failed to read SKILL.md: {e}")))?;

        let (_, body) = extract_frontmatter(&content).unwrap_or(("", content.as_str()));

        let resources = list_skill_resources(&skill.skill_dir);
        let resources_xml = if resources.is_empty() {
            String::new()
        } else {
            let files = resources
                .iter()
                .map(|f| format!("  <file>{}</file>", escape_xml(f)))
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\n<skill_resources>\n{files}\n</skill_resources>")
        };

        Ok(format!(
            "<skill_content name=\"{name}\">\n{body}\n\nSkill directory: {dir}\n\
             Relative paths in this skill are relative to the skill directory.{resources_xml}</skill_content>",
            dir = skill.skill_dir.display(),
        ))
    }
}

fn list_skill_resources(skill_dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for subdir in ["scripts", "references", "assets"] {
        let dir = skill_dir.join(subdir);
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(rel) = path.strip_prefix(skill_dir) {
                        files.push(rel.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, name: &str, description: &str, body: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
        )
        .unwrap();
    }

    #[test]
    fn extract_frontmatter_basic() {
        let content = "---\nname: foo\ndescription: bar\n---\nBody here";
        let (yaml, body) = extract_frontmatter(content).unwrap();
        assert!(yaml.contains("name: foo"));
        assert_eq!(body, "Body here");
    }

    #[test]
    fn extract_frontmatter_missing_returns_none() {
        assert!(extract_frontmatter("no frontmatter").is_none());
    }

    #[test]
    fn discover_skills_finds_valid_skill() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".ccode").join("skills");
        write_skill(&skills_dir, "my-skill-unique-abc123", "Does something useful.", "# Instructions");

        let found = discover_skills(tmp.path());
        let skill = found.iter().find(|s| s.name == "my-skill-unique-abc123");
        assert!(skill.is_some(), "expected to find 'my-skill-unique-abc123'");
        assert_eq!(skill.unwrap().description, "Does something useful.");
    }

    #[test]
    fn discover_skills_skips_missing_description() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".agents").join("skills");
        let skill_dir = skills_dir.join("bad-skill-no-desc-xyz789");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: bad-skill-no-desc-xyz789\n---\nNo description field",
        )
        .unwrap();

        let found = discover_skills(tmp.path());
        assert!(
            !found.iter().any(|s| s.name == "bad-skill-no-desc-xyz789"),
            "skill with missing description should not be discovered"
        );
    }

    #[test]
    fn project_skill_takes_priority_over_user_skill() {
        // We can only test this fully in integration; here just check deduplication
        let tmp = TempDir::new().unwrap();
        let project_skills = tmp.path().join(".ccode").join("skills");
        let agents_skills = tmp.path().join(".agents").join("skills");

        write_skill(&project_skills, "shared-prio-test", "Project version.", "Project body");
        write_skill(&agents_skills, "shared-prio-test", "Agents version.", "Agents body");

        let found = discover_skills(tmp.path());
        let matches: Vec<_> = found.iter().filter(|s| s.name == "shared-prio-test").collect();
        assert_eq!(matches.len(), 1, "should only appear once after deduplication");
        assert_eq!(matches[0].description, "Project version.");
    }

    #[test]
    fn build_skill_catalog_none_when_empty() {
        assert!(build_skill_catalog(&[]).is_none());
    }

    #[test]
    fn build_skill_catalog_contains_name_and_description() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join(".ccode").join("skills");
        write_skill(&skills_dir, "pdf-tool", "Handles PDF files.", "Body");

        let skills = discover_skills(tmp.path());
        let catalog = build_skill_catalog(&skills).unwrap();
        assert!(catalog.contains("<name>pdf-tool</name>"));
        assert!(catalog.contains("<description>Handles PDF files.</description>"));
        assert!(catalog.contains("activate_skill"));
    }

    #[test]
    fn augment_with_skill_catalog_prepends_to_persona() {
        let catalog = Some("CATALOG".to_string());
        let result = augment_with_skill_catalog(Some("Be helpful.".to_string()), &catalog);
        assert_eq!(result.unwrap(), "Be helpful.\n\nCATALOG");
    }

    #[test]
    fn augment_with_skill_catalog_returns_persona_when_no_catalog() {
        let result =
            augment_with_skill_catalog(Some("Be helpful.".to_string()), &None);
        assert_eq!(result.unwrap(), "Be helpful.");
    }
}
