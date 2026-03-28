//! YAML frontmatter parser for markdown reports.

use serde::Deserialize;

/// Parsed frontmatter fields from an agent report.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReportFrontmatter {
    pub agent: Option<String>,
    pub run_id: Option<String>,
    pub prompt_id: Option<String>,
    pub model: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub token_usage: Option<u64>,
    pub findings_count: Option<u32>,
}

/// Split markdown text into optional frontmatter and body.
/// Returns `(Some(frontmatter), body)` if frontmatter exists, else `(None, full text)`.
pub fn parse(text: &str) -> (Option<ReportFrontmatter>, &str) {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return (None, text);
    }

    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    if let Some(end) = after_open.find("\n---") {
        let yaml_str = &after_open[..end];
        let body_start = end + 4; // skip "\n---"
        let body = after_open[body_start..]
            .strip_prefix('\n')
            .unwrap_or(&after_open[body_start..]);

        match serde_yaml::from_str::<ReportFrontmatter>(yaml_str) {
            Ok(frontmatter) => (Some(frontmatter), body),
            Err(_) => (None, text),
        }
    } else {
        (None, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_frontmatter() {
        let input = "---\nagent: codex\nrun_id: mrbl-001\nprompt_id: api-redesign_20260327\n---\n# Report\nContent here";
        let (frontmatter, body) = parse(input);
        let frontmatter = frontmatter.unwrap();

        assert_eq!(frontmatter.agent.as_deref(), Some("codex"));
        assert_eq!(frontmatter.run_id.as_deref(), Some("mrbl-001"));
        assert_eq!(
            frontmatter.prompt_id.as_deref(),
            Some("api-redesign_20260327")
        );
        assert!(body.starts_with("# Report"));
    }

    #[test]
    fn returns_none_for_no_frontmatter() {
        let input = "# Just a report\nNo frontmatter here";
        let (frontmatter, body) = parse(input);

        assert!(frontmatter.is_none());
        assert_eq!(body, input);
    }

    #[test]
    fn handles_malformed_yaml_gracefully() {
        let input = "---\n: this is not valid yaml [\n---\nBody";
        let (frontmatter, _body) = parse(input);

        assert!(frontmatter.is_none());
    }
}
