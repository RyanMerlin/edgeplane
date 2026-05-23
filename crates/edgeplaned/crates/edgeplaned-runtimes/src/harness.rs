//! Shared harness helper — writes the edgeplaned capabilities block to an agent's
//! config file. Idempotent: a marker pair ensures the block appears exactly once.

use anyhow::{Context, Result};
use std::path::Path;

const MARKER_START: &str = "<!-- edgeplaned capabilities -->";
const MARKER_END: &str = "<!-- /edgeplaned capabilities -->";

/// Returns the canonical four-line edgeplane capability block, delimited by markers.
pub fn capabilities_block() -> &'static str {
    "<!-- edgeplaned capabilities -->\n\
## Capabilities\n\
Discover: `edgeplane capabilities [--tag <domain>]`\n\
Detail:   `edgeplane capabilities describe <pack>.<capability>`\n\
Execute:  `edgeplane exec <pack>.<capability> --json [--dry-run]`\n\
History:  `edgeplane receipts last [--json]`\n\
<!-- /edgeplaned capabilities -->"
}

/// Write the capabilities block to `path`, creating parent dirs if needed.
///
/// Idempotent: if the file already contains the marker pair, the existing
/// block is replaced in-place. If the markers are absent, the block is
/// appended (separated by a blank line).
pub fn write_capabilities_block(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }

    let block = capabilities_block();

    if path.exists() {
        let existing = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;

        let start_pos = existing.find(MARKER_START);
        let end_pos = existing.find(MARKER_END);

        let new_content = match (start_pos, end_pos) {
            (Some(s), Some(e)) if s < e => {
                let end_of_block = e + MARKER_END.len();
                format!("{}{}{}", &existing[..s], block, &existing[end_of_block..])
            }
            (None, None) => {
                if existing.ends_with('\n') {
                    format!("{}\n{}\n", existing, block)
                } else {
                    format!("{}\n\n{}\n", existing, block)
                }
            }
            _ => anyhow::bail!(
                "{}: file contains only one edgeplaned marker or markers are inverted; \
                 refusing to modify",
                path.display()
            ),
        };

        std::fs::write(path, new_content)
            .with_context(|| format!("writing {}", path.display()))?;
    } else {
        std::fs::write(path, format!("{}\n", block))
            .with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_capabilities_block_creates_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subdir").join("CLAUDE.md");

        write_capabilities_block(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("edgeplane exec"), "expected 'edgeplane exec' in output:\n{content}");
        assert!(content.contains(MARKER_START));
        assert!(content.contains(MARKER_END));
    }

    #[test]
    fn write_capabilities_block_idempotent_second_write() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");

        write_capabilities_block(&path).unwrap();
        write_capabilities_block(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let count = content.matches(MARKER_START).count();
        assert_eq!(count, 1, "marker appeared {count} times; expected exactly 1");
    }

    #[test]
    fn write_capabilities_block_replaces_existing_block() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");

        std::fs::write(
            &path,
            "# Preamble\n\
             <!-- edgeplaned capabilities -->\n\
             ## Old Content\n\
             <!-- /edgeplaned capabilities -->\n\
             # Postamble\n",
        )
        .unwrap();

        write_capabilities_block(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("edgeplane exec"), "new block not written:\n{content}");
        assert!(content.contains("# Preamble"), "preamble lost");
        assert!(content.contains("# Postamble"), "postamble lost");
        assert!(!content.contains("Old Content"), "old content not removed");
        assert_eq!(content.matches(MARKER_START).count(), 1);
    }
}
