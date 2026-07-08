use crate::state::AppState;
use crate::summary::workflows::models::{Workflow, WorkflowInput};
use crate::summary::workflows::repository::WorkflowsRepository;
use log::{error as log_error, info as log_info};

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
}
