use crate::state::AppState;
use crate::summary::workflows::models::{Workflow, WorkflowInput, WorkflowRun};
use crate::summary::workflows::repository::WorkflowsRepository;
use crate::summary::workflows::runner;
use log::{error as log_error, info as log_info};
use serde::Serialize;
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

#[cfg(test)]
mod tests {
    use crate::summary::workflows::models::{NeoHiveExportConfig, WorkflowInput};
    use crate::summary::workflows::repository::WorkflowsRepository;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

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
