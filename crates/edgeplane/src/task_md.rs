/// TASK.md front-matter (YAML) + markdown body utilities.
///
/// TASK.md is the persistent ABI for cross-runtime task handoff in the mesh.
/// Front-matter carries structured metadata; the body is free-form agent prose.
///
/// Format:
/// ```text
/// ---
/// id: <uuid>
/// title: Task title
/// description: Optional description
/// depends_on: []
/// owner: agent@example.com
/// status: claimed
/// ---
///
/// Free-form markdown the agent reads and writes during execution.
/// ```
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct TaskFrontMatter {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    pub status: String,
}

/// Write TASK.md with YAML front-matter. Body is left empty for the agent to fill.
pub fn write_task_md(path: &Path, fm: &TaskFrontMatter) -> Result<()> {
    let yaml = serde_yaml::to_string(fm)?;
    let content = format!("---\n{}---\n\n", yaml);
    std::fs::write(path, &content)?;
    Ok(())
}

/// Read TASK.md — returns (front_matter, body_text).
pub fn read_task_md(path: &Path) -> Result<(TaskFrontMatter, String)> {
    let content = std::fs::read_to_string(path)?;
    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(sep) = rest.find("\n---\n") {
            let yaml_part = &rest[..sep];
            let body = &rest[sep + 5..];
            let fm: TaskFrontMatter = serde_yaml::from_str(yaml_part)?;
            return Ok((fm, body.to_string()));
        }
    }
    anyhow::bail!("TASK.md at {} is missing YAML front-matter delimiters", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn round_trip_front_matter() {
        let tmp = NamedTempFile::new().unwrap();
        let fm = TaskFrontMatter {
            id: "task-abc".into(),
            title: "Do the thing".into(),
            description: "An important task".into(),
            depends_on: vec!["task-prev".into()],
            owner: "agent@example.com".into(),
            status: "claimed".into(),
        };
        write_task_md(tmp.path(), &fm).unwrap();
        let (parsed, body) = read_task_md(tmp.path()).unwrap();
        assert_eq!(parsed.id, "task-abc");
        assert_eq!(parsed.title, "Do the thing");
        assert_eq!(parsed.depends_on, vec!["task-prev"]);
        assert_eq!(parsed.status, "claimed");
        assert!(body.trim().is_empty()); // body starts empty
    }

    #[test]
    fn read_with_body() {
        use std::io::Write;
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "---").unwrap();
        writeln!(tmp, "id: task-xyz").unwrap();
        writeln!(tmp, "title: Test").unwrap();
        writeln!(tmp, "status: claimed").unwrap();
        writeln!(tmp, "---").unwrap();
        writeln!(tmp, "").unwrap();
        writeln!(tmp, "Agent wrote this result.").unwrap();
        let (parsed, body) = read_task_md(tmp.path()).unwrap();
        assert_eq!(parsed.id, "task-xyz");
        assert!(body.contains("Agent wrote this result."));
    }
}
