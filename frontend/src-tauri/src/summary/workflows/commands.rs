use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::setting::SettingsRepository;
use crate::neohive::NeoHiveClient;
use crate::state::AppState;
use crate::summary::workflows::models::{
    NeoHiveExportConfig, Workflow, WorkflowInput, WorkflowRun, WorkflowRunStatus,
};
use crate::summary::workflows::repository::WorkflowsRepository;
use crate::summary::workflows::runner;
use crate::summary::workflows::sections::{memory_type_for, ParsedSection};
use log::{error as log_error, info as log_info};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime};

#[tauri::command]
pub async fn api_list_workflows(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Workflow>, String> {
    log_info!("api_list_workflows called");
    WorkflowsRepository::list_workflows(state.db_manager.pool())
        .await
        .map_err(|e| {
            log_error!("api_list_workflows failed: {}", e);
            e.to_string()
        })
}

#[tauri::command]
pub async fn api_save_workflow(
    state: tauri::State<'_, AppState>,
    workflow: WorkflowInput,
) -> Result<Workflow, String> {
    log_info!("api_save_workflow called (name: {})", workflow.name);
    if workflow.name.trim().is_empty() {
        return Err("Workflow name cannot be empty".to_string());
    }
    if workflow.provider.trim().is_empty() || workflow.model.trim().is_empty() {
        return Err("Workflow provider and model are required".to_string());
    }
    WorkflowsRepository::upsert_workflow(state.db_manager.pool(), &workflow)
        .await
        .map_err(|e| {
            log_error!("api_save_workflow failed: {}", e);
            e.to_string()
        })
}

#[tauri::command]
pub async fn api_delete_workflow(
    state: tauri::State<'_, AppState>,
    workflow_id: String,
) -> Result<bool, String> {
    log_info!("api_delete_workflow called (id: {})", workflow_id);
    WorkflowsRepository::delete_workflow(state.db_manager.pool(), &workflow_id)
        .await
        .map_err(|e| {
            log_error!("api_delete_workflow failed: {}", e);
            e.to_string()
        })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStartedResponse {
    pub run_id: String,
}

#[tauri::command]
pub async fn api_run_workflow<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    workflow_id: String,
    meeting_id: String,
    text: String,
    summary_language: Option<String>,
) -> Result<RunStartedResponse, String> {
    log_info!("api_run_workflow called (workflow {}, meeting {})", workflow_id, meeting_id);

    let pool = state.db_manager.pool().clone();

    let workflow = WorkflowsRepository::get_workflow(&pool, &workflow_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Workflow '{}' not found", workflow_id))?;

    if text.trim().is_empty() {
        return Err("No transcript text available for this meeting".to_string());
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    WorkflowsRepository::create_run(&pool, &run_id, Some(&workflow.id), &workflow.name, &meeting_id)
        .await
        .map_err(|e| e.to_string())?;

    let summary_language = summary_language.and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t.to_string()) }
    });

    let run_id_spawn = run_id.clone();
    tauri::async_runtime::spawn(async move {
        runner::run_workflow_background(
            app, pool, run_id_spawn, workflow, meeting_id, text, summary_language,
        ).await;
    });

    Ok(RunStartedResponse { run_id })
}

#[tauri::command]
pub async fn api_get_workflow_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<Option<WorkflowRun>, String> {
    WorkflowsRepository::get_run(state.db_manager.pool(), &run_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_list_workflow_runs(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<WorkflowRun>, String> {
    WorkflowsRepository::list_runs_for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_cancel_workflow_run(run_id: String) -> Result<bool, String> {
    log_info!("api_cancel_workflow_run called (run {})", run_id);
    Ok(runner::cancel_run(&run_id))
}

/// One memory to push: (content, memory_type, tags).
#[derive(Debug, PartialEq)]
pub struct ExportItem {
    pub content: String,
    pub mem_type: String,
    pub tags: Vec<String>,
}

/// Pure: turns parsed sections + config + context into export items (skips empty sections).
pub(crate) fn build_export_items(
    sections: &[ParsedSection],
    cfg: &NeoHiveExportConfig,
    meeting_title: &str,
    workflow_name: &str,
) -> Vec<ExportItem> {
    sections
        .iter()
        .filter(|s| !s.content.trim().is_empty())
        .map(|s| ExportItem {
            content: format!("{}\n\n{}", s.title, s.content),
            mem_type: memory_type_for(&s.title, cfg),
            tags: vec![
                meeting_title.to_string(),
                workflow_name.to_string(),
                s.title.clone(),
                "meetily".to_string(),
            ],
        })
        .collect()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub pushed: usize,
    pub failed: usize,
}

/// neohive_status label given attempted export counts (pushed + failed >= 1).
pub(crate) fn neohive_status_label(pushed: usize, failed: usize) -> &'static str {
    if failed == 0 { "pushed" } else if pushed == 0 { "failed" } else { "partial" }
}

/// Exports a completed run's sections to NeoHive. Shared by the manual command
/// and the auto-export hook. Sends over the Cloudflare Access service token.
pub(crate) async fn export_run(pool: &SqlitePool, run_id: &str) -> Result<ExportResult, String> {
    let run = WorkflowsRepository::get_run(pool, run_id)
        .await.map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Run '{}' not found", run_id))?;
    if run.status != WorkflowRunStatus::COMPLETED {
        return Err("Only completed runs can be exported".to_string());
    }

    let sections: Vec<ParsedSection> = run.result_sections.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    // Note: this never trips for a real workflow — `parse_sections` always emits
    // one entry per template section (empty content for missing ones). The real
    // "anything to export" guard is the post-filter `items.is_empty()` check below.
    if sections.is_empty() {
        return Err("This run has no sections to export".to_string());
    }

    let neo = SettingsRepository::get_neohive_config(pool).await.map_err(|e| e.to_string())?;
    if !neo.enabled {
        return Err("NeoHive export is disabled in Settings".to_string());
    }
    let endpoint = neo.endpoint.ok_or("NeoHive endpoint is not configured")?;
    let auth_config_val: serde_json::Value = neo.auth_config.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let auth = crate::neohive::NeoHiveAuth::from_parts(neo.auth_type.as_deref(), &auth_config_val)?;

    // Per-workflow export config (type overrides, importance).
    let export_cfg = match &run.workflow_id {
        Some(id) => WorkflowsRepository::get_workflow(pool, id).await.ok().flatten()
            .map(|w| w.neohive_config()).unwrap_or_default(),
        None => NeoHiveExportConfig::default(),
    };

    // Meeting title for tagging. `MeetingsRepository::get_meeting_by_id` (the brief's
    // guess) does not exist. The real lightweight getter (no transcripts joined) is
    // `get_meeting_metadata`, returning `Result<Option<MeetingModel>, sqlx::Error>`
    // where `MeetingModel.title: String`.
    let meeting_title = MeetingsRepository::get_meeting_metadata(pool, &run.meeting_id)
        .await.ok().flatten().map(|m| m.title).unwrap_or_else(|| "Meeting".to_string());

    let items = build_export_items(&sections, &export_cfg, &meeting_title, &run.workflow_name);
    if items.is_empty() {
        return Err("This run produced no non-empty sections to export".to_string());
    }

    let client = NeoHiveClient::new(endpoint, auth);

    let mut pushed = 0usize;
    let mut failed = 0usize;
    for item in &items {
        match client.store_memory(&item.content, &item.mem_type, &item.tags, export_cfg.importance).await {
            Ok(()) => pushed += 1,
            Err(e) => { log_error!("NeoHive export item failed: {}", e); failed += 1; }
        }
    }

    let status = neohive_status_label(pushed, failed);
    if let Err(e) = WorkflowsRepository::set_run_neohive_status(pool, run_id, status).await {
        log_error!("Failed to update neohive_status for run {}: {}", run_id, e);
    }

    Ok(ExportResult { pushed, failed })
}

#[tauri::command]
pub async fn api_export_run_to_neohive(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<ExportResult, String> {
    log_info!("api_export_run_to_neohive called (run {})", run_id);
    export_run(state.db_manager.pool(), &run_id).await
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeoHiveConfigResponse {
    pub endpoint: Option<String>,
    pub enabled: bool,
    pub auth_type: Option<String>,
    pub auth_config: Option<serde_json::Value>,
}

#[tauri::command]
pub async fn api_get_neohive_config(
    state: tauri::State<'_, AppState>,
) -> Result<NeoHiveConfigResponse, String> {
    log_info!("api_get_neohive_config called");
    let cfg = SettingsRepository::get_neohive_config(state.db_manager.pool())
        .await
        .map_err(|e| { log_error!("api_get_neohive_config failed: {}", e); e.to_string() })?;
    let auth_config = cfg.auth_config.as_deref().and_then(|s| serde_json::from_str(s).ok());
    Ok(NeoHiveConfigResponse {
        endpoint: cfg.endpoint,
        enabled: cfg.enabled,
        auth_type: cfg.auth_type,
        auth_config,
    })
}

#[tauri::command]
pub async fn api_save_neohive_config(
    state: tauri::State<'_, AppState>,
    endpoint: Option<String>,
    enabled: bool,
    auth_type: Option<String>,
    auth_config: Option<serde_json::Value>,
) -> Result<(), String> {
    log_info!("api_save_neohive_config called (enabled={}, authType={:?})", enabled, auth_type);
    let auth_config_str = auth_config.map(|v| v.to_string());
    SettingsRepository::save_neohive_config(
        state.db_manager.pool(),
        endpoint.as_deref(),
        enabled,
        auth_type.as_deref(),
        auth_config_str.as_deref(),
    )
    .await
    .map_err(|e| { log_error!("api_save_neohive_config failed: {}", e); e.to_string() })
}

#[cfg(test)]
mod tests {
    use crate::summary::workflows::models::{NeoHiveExportConfig, WorkflowInput};
    use crate::summary::workflows::repository::WorkflowsRepository;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    #[test]
    fn neohive_status_label_maps_counts() {
        assert_eq!(super::neohive_status_label(3, 0), "pushed");
        assert_eq!(super::neohive_status_label(0, 2), "failed");
        assert_eq!(super::neohive_status_label(2, 1), "partial");
    }

    #[test]
    fn build_export_items_skips_empty_and_maps_types() {
        use crate::summary::workflows::sections::ParsedSection;
        use crate::summary::workflows::models::NeoHiveExportConfig;
        use std::collections::HashMap;
        let mut overrides = HashMap::new();
        overrides.insert("Key Decisions".to_string(), "decision".to_string());
        let cfg = NeoHiveExportConfig { section_type_overrides: overrides, ..Default::default() };
        let sections = vec![
            ParsedSection { title: "Summary".into(), content: "hi".into() },
            ParsedSection { title: "Key Decisions".into(), content: "  ".into() }, // empty -> skipped
        ];
        let items = super::build_export_items(&sections, &cfg, "Sprint", "Exec");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].mem_type, "narrative");
        assert!(items[0].tags.contains(&"meetily".to_string()));
        assert!(items[0].content.starts_with("Summary"));
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().max_connections(1)
            .connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn save_then_list_then_delete_roundtrip() {
        let pool = test_pool().await;
        let input = WorkflowInput {
            id: None, name: "Exec".into(), description: None,
            template_id: "standard_meeting".into(), custom_prompt: None,
            provider: "openrouter".into(), model: "x/y".into(),
            max_tokens: None, temperature: None, top_p: None,
            neohive_export: Some(NeoHiveExportConfig::default()),
        };
        let wf = WorkflowsRepository::upsert_workflow(&pool, &input).await.unwrap();
        assert_eq!(WorkflowsRepository::list_workflows(&pool).await.unwrap().len(), 1);
        assert!(WorkflowsRepository::delete_workflow(&pool, &wf.id).await.unwrap());
        assert_eq!(WorkflowsRepository::list_workflows(&pool).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn create_run_shows_in_meeting_run_list() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        WorkflowsRepository::create_run(&pool, "run1", None, "W", "m1").await.unwrap();
        let runs = WorkflowsRepository::list_runs_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "queued");
        assert_eq!(runs[0].neohive_status, "none");
    }
}
