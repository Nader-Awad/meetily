//! Custom vocabulary: a two-entry-type dictionary (Terms + Corrections) plus the
//! pure logic that consumes it. Terms bias Whisper (`initial_prompt`); Corrections
//! are deterministic post-ASR find-and-replace; descriptions feed a summary glossary.
//! This module is pure and has no I/O — persistence lives in the settings repository.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// One dictionary entry. `entry_type` is "term" (bias the recognizer) or
/// "correction" (replace `text` with `replacement`). `description` is optional
/// and, when present, feeds the summarization glossary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VocabularyEntry {
    pub id: String,
    #[serde(rename = "entryType")]
    pub entry_type: String,
    pub text: String,
    #[serde(default)]
    pub replacement: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "caseSensitive", default)]
    pub case_sensitive: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<String>,
}

/// The whole persisted vocabulary. `enabled` is a global master switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VocabularyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub entries: Vec<VocabularyEntry>,
}

impl Default for VocabularyConfig {
    fn default() -> Self {
        Self { enabled: true, entries: Vec::new() }
    }
}

/// A resolved correction ready to apply.
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    pub from: String,
    pub to: String,
    pub case_sensitive: bool,
}

impl VocabularyConfig {
    /// Enabled correction entries that have a replacement. Empty if the master
    /// switch is off.
    pub fn corrections(&self) -> Vec<Correction> {
        if !self.enabled {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.enabled && e.entry_type == "correction" && !e.text.trim().is_empty())
            .filter_map(|e| {
                e.replacement.as_ref().map(|r| Correction {
                    from: e.text.clone(),
                    to: r.clone(),
                    case_sensitive: e.case_sensitive,
                })
            })
            .collect()
    }

    /// Enabled term texts (for Whisper biasing). Empty if the master switch is off.
    pub fn term_texts(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.enabled && e.entry_type == "term" && !e.text.trim().is_empty())
            .map(|e| e.text.clone())
            .collect()
    }

    /// (term, description) for every enabled entry that has a non-empty
    /// description (for the summary glossary). Empty if the master switch is off.
    pub fn glossary_entries(&self) -> Vec<(String, String)> {
        if !self.enabled {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.enabled && !e.text.trim().is_empty())
            .filter_map(|e| {
                e.description
                    .as_ref()
                    .map(|d| d.trim())
                    .filter(|d| !d.is_empty())
                    .map(|d| (e.text.clone(), d.to_string()))
            })
            .collect()
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whole-word, boundary-aware replacement of `from` with `to`. Case-insensitive
/// unless `case_sensitive`. ASCII case folding only (fine for proper nouns);
/// non-ASCII case differences won't match unless `case_sensitive` is false and
/// the letters are ASCII. Left-to-right, non-overlapping.
fn replace_whole_word(haystack: &str, from: &str, to: &str, case_sensitive: bool) -> String {
    if from.is_empty() {
        return haystack.to_string();
    }
    let hay: Vec<char> = haystack.chars().collect();
    let pat: Vec<char> = from.chars().collect();
    let plen = pat.len();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while i < hay.len() {
        let end = i + plen;
        let is_match = end <= hay.len()
            && hay[i..end].iter().zip(pat.iter()).all(|(a, b)| {
                if case_sensitive {
                    a == b
                } else {
                    a.eq_ignore_ascii_case(b)
                }
            });
        let left_ok = i == 0 || !is_word_char(hay[i - 1]);
        let right_ok = end >= hay.len() || !is_word_char(hay[end]);
        if is_match && left_ok && right_ok {
            out.push_str(to);
            i = end;
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// Apply every correction in order to `text`. Corrections are applied
/// sequentially against the accumulating result (not the original input), so
/// a later correction can re-match text produced by an earlier one. This is
/// intentional cascading behavior, not a bug — do not "fix" it by applying
/// corrections independently against the original `text`.
pub fn apply_corrections(text: &str, corrections: &[Correction]) -> String {
    let mut result = text.to_string();
    for c in corrections {
        result = replace_whole_word(&result, &c.from, &c.to, c.case_sensitive);
    }
    result
}

/// Build a Whisper `initial_prompt` from term texts: comma-joined, `\0`-stripped,
/// clipped to `max_chars` on a term boundary. Empty when there are no terms.
pub fn build_whisper_prompt(terms: &[String], max_chars: usize) -> String {
    let mut out = String::new();
    for t in terms {
        let cleaned = t.replace('\0', "");
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        let candidate = if out.is_empty() {
            cleaned.to_string()
        } else {
            format!("{}, {}", out, cleaned)
        };
        if candidate.chars().count() > max_chars {
            break;
        }
        out = candidate;
    }
    out
}

/// Build a glossary block for the summarization prompt. Empty when no described
/// terms. Clipped to `max_chars` on a line boundary.
pub fn build_glossary(entries: &[(String, String)], max_chars: usize) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let header = "Glossary of domain terms (use these exact spellings; do not confuse them):\n";
    let mut body = String::from(header);
    for (term, desc) in entries {
        let line = format!("- {}: {}\n", term.trim(), desc.trim());
        if body.chars().count() + line.chars().count() > max_chars {
            break;
        }
        body.push_str(&line);
    }
    // If nothing but the header fit, treat as empty.
    if body == header {
        return String::new();
    }
    body
}

/// Prepend a glossary (built from `entries`) to `custom_prompt`. Returns the
/// prompt unchanged when there are no described terms; returns the glossary
/// alone when `custom_prompt` is blank.
pub fn prepend_glossary(custom_prompt: String, entries: &[(String, String)], max_chars: usize) -> String {
    let glossary = build_glossary(entries, max_chars);
    if glossary.is_empty() {
        custom_prompt
    } else if custom_prompt.trim().is_empty() {
        glossary
    } else {
        format!("{}\n\n{}", glossary, custom_prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corr(from: &str, to: &str, cs: bool) -> Correction {
        Correction { from: from.into(), to: to.into(), case_sensitive: cs }
    }

    #[test]
    fn corrects_case_insensitive_whole_word() {
        let c = vec![corr("sneak", "Snyk", false)];
        assert_eq!(apply_corrections("I ran a sneak scan", &c), "I ran a Snyk scan");
        assert_eq!(apply_corrections("Sneak found bugs", &c), "Snyk found bugs");
    }

    #[test]
    fn does_not_touch_substrings() {
        let c = vec![corr("sneak", "Snyk", false)];
        assert_eq!(apply_corrections("that was sneaky", &c), "that was sneaky");
    }

    #[test]
    fn preserves_adjacent_punctuation() {
        let c = vec![corr("sneak", "Snyk", false)];
        assert_eq!(apply_corrections("sneak, then done", &c), "Snyk, then done");
    }

    #[test]
    fn case_sensitive_only_matches_exact() {
        let c = vec![corr("OSS", "OSS project", true)];
        assert_eq!(apply_corrections("the oss thing", &c), "the oss thing");
        assert_eq!(apply_corrections("the OSS thing", &c), "the OSS project thing");
    }

    #[test]
    fn multi_word_from() {
        let c = vec![corr("near hive", "NeoHive", false)];
        assert_eq!(apply_corrections("the near hive system", &c), "the NeoHive system");
    }

    #[test]
    fn empty_from_is_noop() {
        let c = vec![corr("", "x", false)];
        assert_eq!(apply_corrections("hello", &c), "hello");
    }

    #[test]
    fn apply_corrections_cascades_across_entries() {
        // Intentional: corrections apply sequentially against the accumulating
        // result, so a later correction can re-match an earlier one's output.
        let c = vec![corr("x", "y", false), corr("y", "z", false)];
        assert_eq!(apply_corrections("x", &c), "z");
    }

    #[test]
    fn whisper_prompt_joins_and_clips() {
        let terms = vec!["Snyk".to_string(), "NeoHive".to_string(), "Logilica".to_string()];
        assert_eq!(build_whisper_prompt(&terms, 100), "Snyk, NeoHive, Logilica");
        // clip: only "Snyk" fits before the next term would exceed 6 chars
        assert_eq!(build_whisper_prompt(&terms, 6), "Snyk");
    }

    #[test]
    fn whisper_prompt_strips_null_bytes_and_blanks() {
        let terms = vec!["Sn\0yk".to_string(), "   ".to_string(), "NeoHive".to_string()];
        assert_eq!(build_whisper_prompt(&terms, 100), "Snyk, NeoHive");
    }

    #[test]
    fn glossary_builds_and_skips_when_empty() {
        assert_eq!(build_glossary(&[], 1000), "");
        let e = vec![("Snyk".to_string(), "SAST company".to_string())];
        let g = build_glossary(&e, 1000);
        assert!(g.contains("Glossary of domain terms"));
        assert!(g.contains("- Snyk: SAST company"));
    }

    #[test]
    fn glossary_clips_at_line_boundary() {
        let header = "Glossary of domain terms (use these exact spellings; do not confuse them):\n";
        let entries = vec![
            ("Term1".to_string(), "Desc1".to_string()),
            ("Term2".to_string(), "Desc2".to_string()),
        ];
        let line1 = format!("- {}: {}\n", "Term1", "Desc1");
        // Exactly enough room for header + first line, not the second.
        let max_chars = header.chars().count() + line1.chars().count();
        let g = build_glossary(&entries, max_chars);
        assert!(g.contains("Term1"));
        assert!(!g.contains("Term2"));
    }

    #[test]
    fn glossary_returns_empty_when_only_header_fits() {
        let header = "Glossary of domain terms (use these exact spellings; do not confuse them):\n";
        let entries = vec![("Term1".to_string(), "Desc1".to_string())];
        // Room for the header alone, but not header + first line.
        let max_chars = header.chars().count();
        assert_eq!(build_glossary(&entries, max_chars), "");
    }

    #[test]
    fn prepend_glossary_returns_prompt_unchanged_when_no_described_terms() {
        assert_eq!(prepend_glossary("keep this prompt".to_string(), &[], 2000), "keep this prompt");
    }

    #[test]
    fn prepend_glossary_returns_glossary_only_when_prompt_blank() {
        let entries = vec![("Snyk".to_string(), "SAST company".to_string())];
        let result = prepend_glossary("   ".to_string(), &entries, 2000);
        assert!(result.starts_with("Glossary of domain terms"));
        assert!(result.contains("- Snyk: SAST company"));
    }

    #[test]
    fn prepend_glossary_combines_glossary_and_prompt_glossary_first() {
        let entries = vec![("Snyk".to_string(), "SAST company".to_string())];
        let result = prepend_glossary("Focus on action items.".to_string(), &entries, 2000);
        assert!(result.contains("Glossary of domain terms"));
        assert!(result.contains("- Snyk: SAST company"));
        assert!(result.contains("Focus on action items."));
        let glossary_pos = result.find("Glossary of domain terms").unwrap();
        let prompt_pos = result.find("Focus on action items.").unwrap();
        assert!(glossary_pos < prompt_pos);
    }

    #[test]
    fn config_derivations_respect_master_switch_and_types() {
        let json = r#"{
          "enabled": true,
          "entries": [
            {"id":"1","entryType":"term","text":"Snyk","description":"SAST company","enabled":true},
            {"id":"2","entryType":"correction","text":"sneak","replacement":"Snyk","enabled":true},
            {"id":"3","entryType":"correction","text":"off","replacement":"OFF","enabled":false}
          ]
        }"#;
        let cfg: VocabularyConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.term_texts(), vec!["Snyk".to_string()]);
        assert_eq!(cfg.corrections().len(), 1);
        assert_eq!(cfg.corrections()[0].to, "Snyk");
        assert_eq!(cfg.glossary_entries(), vec![("Snyk".to_string(), "SAST company".to_string())]);

        let mut off = cfg.clone();
        off.enabled = false;
        assert!(off.term_texts().is_empty());
        assert!(off.corrections().is_empty());
        assert!(off.glossary_entries().is_empty());
    }

    #[test]
    fn corrections_excludes_whitespace_only_text() {
        let json = r#"{
          "enabled": true,
          "entries": [
            {"id":"1","entryType":"correction","text":"   ","replacement":"X","enabled":true}
          ]
        }"#;
        let cfg: VocabularyConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.corrections().is_empty());
        assert_eq!(
            apply_corrections("some   text here", &cfg.corrections()),
            "some   text here"
        );
    }

    #[test]
    fn glossary_entries_excludes_whitespace_only_text() {
        let json = r#"{
          "enabled": true,
          "entries": [
            {"id":"1","entryType":"term","text":"   ","description":"desc","enabled":true}
          ]
        }"#;
        let cfg: VocabularyConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.glossary_entries().is_empty());
    }

    #[test]
    fn config_serde_round_trips_camel_case() {
        let cfg = VocabularyConfig {
            enabled: true,
            entries: vec![VocabularyEntry {
                id: "1".into(),
                entry_type: "correction".into(),
                text: "sneak".into(),
                replacement: Some("Snyk".into()),
                description: None,
                case_sensitive: false,
                enabled: true,
                created_at: Some("2024-01-01T00:00:00Z".into()),
                updated_at: Some("2024-01-02T00:00:00Z".into()),
            }],
        };
        let s = serde_json::to_string(&cfg).unwrap();
        assert!(s.contains("\"entryType\""));
        assert!(s.contains("\"caseSensitive\""));
        assert!(s.contains("\"createdAt\""));
        assert!(s.contains("\"updatedAt\""));
        let back: VocabularyConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].entry_type, "correction");
        assert_eq!(back.entries[0].created_at, Some("2024-01-01T00:00:00Z".to_string()));
        assert_eq!(back.entries[0].updated_at, Some("2024-01-02T00:00:00Z".to_string()));
    }
}
