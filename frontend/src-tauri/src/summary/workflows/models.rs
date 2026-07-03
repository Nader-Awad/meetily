use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Section-title -> memory-type overrides for NeoHive export.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeoHiveExportConfig {
    /// Whether this workflow may export to NeoHive at all.
    #[serde(default)]
    pub enabled: bool,
    /// If true, export automatically when a run completes; else manual button only.
    #[serde(default)]
    pub auto_export: bool,
    /// Per-section-title memory type override (e.g. "Key Decisions" -> "decision").
    #[serde(default)]
    pub section_type_overrides: HashMap<String, String>,
    /// Memory type for sections without an override.
    #[serde(default = "default_memory_type")]
    pub default_type: String,
    /// Importance (1-10) applied to every exported memory.
    #[serde(default = "default_importance")]
    pub importance: u8,
}

fn default_memory_type() -> String { "narrative".to_string() }
fn default_importance() -> u8 { 6 }

impl Default for NeoHiveExportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_export: false,
            section_type_overrides: HashMap::new(),
            default_type: default_memory_type(),
            importance: default_importance(),
        }
    }
}

/// A saved workflow recipe (DB row). DB columns are snake_case; JSON to the
/// frontend is camelCase.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub template_id: String,
    pub custom_prompt: Option<String>,
    pub provider: String,
    pub model: String,
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    /// Raw JSON string of NeoHiveExportConfig; parse with `neohive_config()`.
    pub neohive_export: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Workflow {
    /// Parses the stored export config, falling back to a disabled default.
    pub fn neohive_config(&self) -> NeoHiveExportConfig {
        self.neohive_export
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

/// Frontend-supplied workflow definition for create/update.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInput {
    /// None => create; Some => update existing.
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub template_id: String,
    pub custom_prompt: Option<String>,
    pub provider: String,
    pub model: String,
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub neohive_export: Option<NeoHiveExportConfig>,
}

/// A workflow run (DB row / poll target).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: Option<String>,
    pub workflow_name: String,
    pub meeting_id: String,
    pub status: String,
    pub result_markdown: Option<String>,
    /// JSON array of { title, content }.
    pub result_sections: Option<String>,
    pub error: Option<String>,
    pub neohive_status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Terminal + in-flight statuses as string constants (kept as &str to match the
/// TEXT column and the frontend polling contract).
pub struct WorkflowRunStatus;
impl WorkflowRunStatus {
    pub const QUEUED: &'static str = "queued";
    pub const RUNNING: &'static str = "running";
    pub const COMPLETED: &'static str = "completed";
    pub const ERROR: &'static str = "error";
    pub const CANCELLED: &'static str = "cancelled";
}
