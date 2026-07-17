use super::defaults;
use super::types::Template;
use std::path::Path;
use std::path::PathBuf;
use tracing::{debug, info, warn};
use once_cell::sync::Lazy;
use std::sync::RwLock;

// Global storage for the bundled templates directory path
static BUNDLED_TEMPLATES_DIR: Lazy<RwLock<Option<PathBuf>>> = Lazy::new(|| RwLock::new(None));

/// Set the bundled templates directory path (called once at app startup)
pub fn set_bundled_templates_dir(path: PathBuf) {
    info!("Bundled templates directory set to: {:?}", path);
    if let Ok(mut dir) = BUNDLED_TEMPLATES_DIR.write() {
        *dir = Some(path);
    }
}

/// Get the user's custom templates directory path
///
/// Returns the platform-specific application data directory for custom templates:
/// - macOS: ~/Library/Application Support/Meetily/templates/
/// - Windows: %APPDATA%\Meetily\templates\
/// - Linux: ~/.config/Meetily/templates/
fn get_custom_templates_dir() -> Option<PathBuf> {
    let mut path = dirs::data_dir()?;
    path.push("Meetily");
    path.push("templates");
    Some(path)
}

/// Valid custom-template id = filename stem. Restricted to prevent path traversal.
pub fn is_valid_template_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// True if a user-authored custom template file exists for this id.
pub fn is_custom_template(id: &str) -> bool {
    get_custom_templates_dir()
        .map(|d| d.join(format!("{id}.json")).exists())
        .unwrap_or(false)
}

/// Testable core: write `<dir>/<id>.json` (creating `dir` if needed).
pub fn save_custom_template_in(dir: &Path, id: &str, template: &Template) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create templates dir: {e}"))?;
    let json = serde_json::to_string_pretty(template).map_err(|e| format!("Failed to serialize template: {e}"))?;
    std::fs::write(dir.join(format!("{id}.json")), json).map_err(|e| format!("Failed to write template '{id}': {e}"))?;
    Ok(())
}

/// Write a custom template into the user's custom templates directory.
pub fn save_custom_template(id: &str, template: &Template) -> Result<(), String> {
    let dir = get_custom_templates_dir().ok_or_else(|| "Could not resolve custom templates directory".to_string())?;
    save_custom_template_in(&dir, id, template)
}

/// Testable core: remove `<dir>/<id>.json`; err if absent.
pub fn delete_custom_template_in(dir: &Path, id: &str) -> Result<(), String> {
    let path = dir.join(format!("{id}.json"));
    if !path.exists() {
        return Err(format!("Custom template '{id}' does not exist"));
    }
    std::fs::remove_file(&path).map_err(|e| format!("Failed to delete template '{id}': {e}"))
}

/// Delete a custom template from the user's custom templates directory.
pub fn delete_custom_template(id: &str) -> Result<(), String> {
    let dir = get_custom_templates_dir().ok_or_else(|| "Could not resolve custom templates directory".to_string())?;
    delete_custom_template_in(&dir, id)
}

/// Load a template from the bundled resources directory
///
/// # Arguments
/// * `template_id` - Template identifier (without .json extension)
///
/// # Returns
/// The template JSON content if found, None otherwise
fn load_bundled_template(template_id: &str) -> Option<String> {
    let bundled_dir = BUNDLED_TEMPLATES_DIR.read().ok()?.clone()?;
    let template_path = bundled_dir.join(format!("{}.json", template_id));

    debug!("Checking for bundled template at: {:?}", template_path);

    match std::fs::read_to_string(&template_path) {
        Ok(content) => {
            info!("Loaded bundled template '{}' from {:?}", template_id, template_path);
            Some(content)
        }
        Err(e) => {
            debug!("No bundled template '{}' found: {}", template_id, e);
            None
        }
    }
}

/// Load a template from the user's custom templates directory
///
/// # Arguments
/// * `template_id` - Template identifier (without .json extension)
///
/// # Returns
/// The template JSON content if found, None otherwise
fn load_custom_template(template_id: &str) -> Option<String> {
    let custom_dir = get_custom_templates_dir()?;
    let template_path = custom_dir.join(format!("{}.json", template_id));

    debug!("Checking for custom template at: {:?}", template_path);

    match std::fs::read_to_string(&template_path) {
        Ok(content) => {
            info!("Loaded custom template '{}' from {:?}", template_id, template_path);
            Some(content)
        }
        Err(e) => {
            debug!("No custom template '{}' found: {}", template_id, e);
            None
        }
    }
}

/// Load and parse a template by identifier
///
/// This function implements a fallback strategy:
/// 1. Check user's custom templates directory
/// 2. Check bundled resources directory (app templates)
/// 3. Fall back to built-in embedded templates
/// 4. Return error if not found in any location
///
/// # Arguments
/// * `template_id` - Template identifier (e.g., "daily_standup", "standard_meeting")
///
/// # Returns
/// Parsed and validated Template struct
pub fn get_template(template_id: &str) -> Result<Template, String> {
    info!("Loading template: {}", template_id);

    // Try custom template first, then bundled, then built-in
    let json_content = if let Some(custom_content) = load_custom_template(template_id) {
        debug!("Using custom template for '{}'", template_id);
        custom_content
    } else if let Some(bundled_content) = load_bundled_template(template_id) {
        debug!("Using bundled template for '{}'", template_id);
        bundled_content
    } else if let Some(builtin_content) = defaults::get_builtin_template(template_id) {
        debug!("Using built-in template for '{}'", template_id);
        builtin_content.to_string()
    } else {
        return Err(format!(
            "Template '{}' not found. Available templates: {}",
            template_id,
            list_template_ids().join(", ")
        ));
    };

    // Parse and validate
    validate_and_parse_template(&json_content)
}

/// Validate and parse template JSON
///
/// # Arguments
/// * `json_content` - Raw JSON string
///
/// # Returns
/// Parsed and validated Template struct
pub fn validate_and_parse_template(json_content: &str) -> Result<Template, String> {
    let template: Template = serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse template JSON: {}", e))?;

    template.validate()?;

    Ok(template)
}

/// List all available template identifiers
///
/// Returns a combined list of:
/// - Built-in template IDs
/// - Bundled template IDs (from app resources)
/// - Custom template IDs (from user's data directory)
pub fn list_template_ids() -> Vec<String> {
    let mut ids: Vec<String> = defaults::list_builtin_template_ids()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    // Add bundled templates if directory is set
    if let Ok(bundled_dir_lock) = BUNDLED_TEMPLATES_DIR.read() {
        if let Some(bundled_dir) = bundled_dir_lock.as_ref() {
            if bundled_dir.exists() {
                match std::fs::read_dir(bundled_dir) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            if let Some(filename) = entry.file_name().to_str() {
                                if filename.ends_with(".json") {
                                    let id = filename.trim_end_matches(".json").to_string();
                                    if !ids.contains(&id) {
                                        ids.push(id);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read bundled templates directory: {}", e);
                    }
                }
            }
        }
    }

    // Add custom templates if directory exists
    if let Some(custom_dir) = get_custom_templates_dir() {
        if custom_dir.exists() {
            match std::fs::read_dir(&custom_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        if let Some(filename) = entry.file_name().to_str() {
                            if filename.ends_with(".json") {
                                let id = filename.trim_end_matches(".json").to_string();
                                if !ids.contains(&id) {
                                    ids.push(id);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read custom templates directory: {}", e);
                }
            }
        }
    }

    ids.sort();
    ids
}

/// List all available templates with their metadata
///
/// Returns a list of (id, name, description) tuples
pub fn list_templates() -> Vec<(String, String, String)> {
    let mut templates = Vec::new();

    for id in list_template_ids() {
        match get_template(&id) {
            Ok(template) => {
                templates.push((id, template.name, template.description));
            }
            Err(e) => {
                warn!("Failed to load template '{}': {}", id, e);
            }
        }
    }

    templates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_builtin_template() {
        let template = get_template("daily_standup");
        assert!(template.is_ok());

        let template = template.unwrap();
        assert_eq!(template.name, "Daily Standup");
        assert!(!template.sections.is_empty());
    }

    #[test]
    fn test_get_nonexistent_template() {
        let result = get_template("nonexistent_template");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_template_ids() {
        let ids = list_template_ids();
        assert!(ids.contains(&"daily_standup".to_string()));
        assert!(ids.contains(&"standard_meeting".to_string()));
    }

    #[test]
    fn test_validate_invalid_json() {
        let result = validate_and_parse_template("invalid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_new_builtin_templates_are_valid() {
        for content in [
            include_str!("../../../templates/technical_decisions.json"),
            include_str!("../../../templates/action_items.json"),
            include_str!("../../../templates/comprehensive_meeting.json"),
        ] {
            let template = validate_and_parse_template(content).expect("template should be valid");
            assert!(!template.name.is_empty());
            assert!(!template.sections.is_empty());
        }
    }

    #[test]
    fn valid_template_id_rules() {
        assert!(is_valid_template_id("daily_standup"));
        assert!(is_valid_template_id("my-tpl-1"));
        assert!(!is_valid_template_id(""));
        assert!(!is_valid_template_id("../evil"));
        assert!(!is_valid_template_id("a/b"));
        assert!(!is_valid_template_id("Has Space"));
        assert!(!is_valid_template_id("UPPER"));
        assert!(!is_valid_template_id(&"x".repeat(65)));
    }

    #[test]
    fn custom_template_save_read_delete_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("meetily_tpl_test_{}", uuid::Uuid::new_v4()));
        let json = r#"{"name":"T","description":"d","sections":[{"title":"S","instruction":"i","format":"list"}]}"#;
        let tpl = validate_and_parse_template(json).expect("valid");
        save_custom_template_in(&tmp, "my_tpl", &tpl).expect("save ok (creates dir)");
        let content = std::fs::read_to_string(tmp.join("my_tpl.json")).expect("file written");
        let parsed = validate_and_parse_template(&content).expect("round-trips + validates");
        assert_eq!(parsed.name, "T");
        assert_eq!(parsed.sections.len(), 1);
        delete_custom_template_in(&tmp, "my_tpl").expect("delete ok");
        assert!(!tmp.join("my_tpl.json").exists());
        assert!(delete_custom_template_in(&tmp, "my_tpl").is_err()); // gone now
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
