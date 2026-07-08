use crate::summary::workflows::models::NeoHiveExportConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedSection {
    pub title: String,
    pub content: String,
}

/// Normalizes a line for heading comparison: strips leading `#`, surrounding
/// `**`, whitespace, and a trailing `:`, then lowercases.
fn normalize_heading(line: &str) -> String {
    let mut s = line.trim();
    // strip markdown heading hashes
    s = s.trim_start_matches('#').trim();
    // strip bold markers
    if let Some(inner) = s.strip_prefix("**").and_then(|x| x.strip_suffix("**")) {
        s = inner.trim();
    }
    let s = s.trim_end_matches(':').trim();
    s.to_lowercase()
}

/// Returns true if `line` is a heading for `title` (bold, #, or plain), tolerant
/// of case and a trailing colon.
fn is_heading_for(line: &str, title_lower: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    // Only treat short-ish lines as potential headings to avoid matching prose
    // that merely mentions the title.
    let looks_like_heading = t.starts_with('#')
        || (t.starts_with("**") && t.trim_end_matches(':').trim_end().ends_with("**"))
        || t.trim_end_matches(':').eq_ignore_ascii_case(title_lower); // plain title-only line
    looks_like_heading && normalize_heading(t) == title_lower
}

/// Splits the generated markdown into sections in the order of `section_titles`.
/// Content of each section is everything between its heading and the next
/// recognized section heading (or end of document). Missing sections get "".
pub fn parse_sections(markdown: &str, section_titles: &[String]) -> Vec<ParsedSection> {
    let lines: Vec<&str> = markdown.lines().collect();
    let titles_lower: Vec<String> = section_titles.iter().map(|t| t.trim().to_lowercase()).collect();

    // For each line index, if it is a heading for some known title, record (line_idx, title_idx).
    let mut markers: Vec<(usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        for (ti, tl) in titles_lower.iter().enumerate() {
            if is_heading_for(line, tl) {
                markers.push((i, ti));
                break;
            }
        }
    }

    // Content for a title = lines after its heading up to the next marker line.
    let mut content_by_title: Vec<String> = vec![String::new(); section_titles.len()];
    for (m_idx, &(line_idx, title_idx)) in markers.iter().enumerate() {
        let start = line_idx + 1;
        let end = markers
            .get(m_idx + 1)
            .map(|&(next_line, _)| next_line)
            .unwrap_or(lines.len());
        if start <= end {
            let body = lines[start..end].join("\n").trim().to_string();
            // Last marker for a given title wins only if earlier was empty; keep first non-empty.
            if content_by_title[title_idx].is_empty() {
                content_by_title[title_idx] = body;
            }
        }
    }

    section_titles
        .iter()
        .enumerate()
        .map(|(i, title)| ParsedSection {
            title: title.clone(),
            content: content_by_title[i].clone(),
        })
        .collect()
}

/// Resolves the NeoHive memory type for a section: explicit override wins, else default.
pub fn memory_type_for(section_title: &str, cfg: &NeoHiveExportConfig) -> String {
    cfg.section_type_overrides
        .get(section_title)
        .cloned()
        .unwrap_or_else(|| cfg.default_type.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::workflows::models::NeoHiveExportConfig;
    use std::collections::HashMap;

    fn titles() -> Vec<String> {
        vec!["Summary".into(), "Key Decisions".into(), "Action Items".into()]
    }

    #[test]
    fn parses_bold_delimited_sections() {
        let md = "# Team Sync\n\n**Summary**\n\nWe shipped v2.\n\n**Key Decisions**\n\n- Ship Friday\n\n**Action Items**\n\n- Alice: docs\n";
        let out = parse_sections(md, &titles());
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].title, "Summary");
        assert!(out[0].content.contains("We shipped v2."));
        assert!(!out[0].content.contains("Key Decisions")); // does not bleed into next
        assert!(out[1].content.contains("Ship Friday"));
        assert!(out[2].content.contains("Alice: docs"));
    }

    #[test]
    fn parses_hash_heading_sections() {
        let md = "## Summary\nAll good.\n## Key Decisions\nNone.\n## Action Items\nNone.\n";
        let out = parse_sections(md, &titles());
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].content.trim(), "All good.");
        assert_eq!(out[1].content.trim(), "None.");
    }

    #[test]
    fn missing_section_yields_empty_content_and_is_kept_in_order() {
        let md = "**Summary**\nHi.\n**Action Items**\n- x\n"; // no "Key Decisions"
        let out = parse_sections(md, &titles());
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].title, "Key Decisions");
        assert_eq!(out[1].content.trim(), "");
        assert!(out[2].content.contains("- x"));
    }

    #[test]
    fn heading_matches_are_case_and_colon_insensitive() {
        let md = "**summary:**\nlower\n**KEY DECISIONS**\nupper\n**Action Items**\nok\n";
        let out = parse_sections(md, &titles());
        assert_eq!(out[0].content.trim(), "lower");
        assert_eq!(out[1].content.trim(), "upper");
    }

    #[test]
    fn memory_type_uses_override_then_default() {
        let mut overrides = HashMap::new();
        overrides.insert("Key Decisions".to_string(), "decision".to_string());
        let cfg = NeoHiveExportConfig { section_type_overrides: overrides, default_type: "narrative".into(), ..Default::default() };
        assert_eq!(memory_type_for("Key Decisions", &cfg), "decision");
        assert_eq!(memory_type_for("Summary", &cfg), "narrative");
    }
}
