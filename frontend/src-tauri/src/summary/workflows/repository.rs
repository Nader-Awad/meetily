use crate::summary::workflows::models::{Workflow, WorkflowInput, WorkflowRun};
use sqlx::SqlitePool;

pub struct WorkflowsRepository;

impl WorkflowsRepository {
    pub async fn list_workflows(pool: &SqlitePool) -> Result<Vec<Workflow>, sqlx::Error> {
        sqlx::query_as::<_, Workflow>("SELECT * FROM workflows ORDER BY updated_at DESC")
            .fetch_all(pool)
            .await
    }

    pub async fn get_workflow(pool: &SqlitePool, id: &str) -> Result<Option<Workflow>, sqlx::Error> {
        sqlx::query_as::<_, Workflow>("SELECT * FROM workflows WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn upsert_workflow(
        pool: &SqlitePool,
        input: &WorkflowInput,
    ) -> Result<Workflow, sqlx::Error> {
        let now = chrono::Utc::now();
        let id = input.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let export_json = match &input.neohive_export {
            Some(cfg) => Some(
                serde_json::to_string(cfg)
                    .map_err(|e| sqlx::Error::Protocol(format!("serialize export cfg: {}", e).into()))?,
            ),
            None => None,
        };

        sqlx::query(
            r#"
            INSERT INTO workflows
                (id, name, description, template_id, custom_prompt, provider, model,
                 max_tokens, temperature, top_p, neohive_export, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                template_id = excluded.template_id,
                custom_prompt = excluded.custom_prompt,
                provider = excluded.provider,
                model = excluded.model,
                max_tokens = excluded.max_tokens,
                temperature = excluded.temperature,
                top_p = excluded.top_p,
                neohive_export = excluded.neohive_export,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.template_id)
        .bind(&input.custom_prompt)
        .bind(&input.provider)
        .bind(&input.model)
        .bind(input.max_tokens)
        .bind(input.temperature)
        .bind(input.top_p)
        .bind(&export_json)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        Self::get_workflow(pool, &id)
            .await?
            .ok_or_else(|| sqlx::Error::RowNotFound)
    }

    pub async fn delete_workflow(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM workflows WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn create_run(
        pool: &SqlitePool,
        run_id: &str,
        workflow_id: Option<&str>,
        workflow_name: &str,
        meeting_id: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query(
            r#"
            INSERT INTO workflow_runs
                (id, workflow_id, workflow_name, meeting_id, status, neohive_status, created_at, updated_at)
            VALUES (?, ?, ?, ?, 'queued', 'none', ?, ?)
            "#,
        )
        .bind(run_id)
        .bind(workflow_id)
        .bind(workflow_name)
        .bind(meeting_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn complete_run(
        pool: &SqlitePool,
        run_id: &str,
        markdown: &str,
        sections_json: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query(
            r#"
            UPDATE workflow_runs
            SET status = ?, result_markdown = ?, result_sections = ?, error = NULL, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(markdown)
        .bind(sections_json)
        .bind(now)
        .bind(run_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn fail_run(
        pool: &SqlitePool,
        run_id: &str,
        error: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query("UPDATE workflow_runs SET status = ?, error = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(error)
            .bind(now)
            .bind(run_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn get_run(pool: &SqlitePool, run_id: &str) -> Result<Option<WorkflowRun>, sqlx::Error> {
        sqlx::query_as::<_, WorkflowRun>("SELECT * FROM workflow_runs WHERE id = ?")
            .bind(run_id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_runs_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<WorkflowRun>, sqlx::Error> {
        sqlx::query_as::<_, WorkflowRun>(
            "SELECT * FROM workflow_runs WHERE meeting_id = ? ORDER BY created_at DESC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    pub async fn set_run_neohive_status(
        pool: &SqlitePool,
        run_id: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now();
        sqlx::query("UPDATE workflow_runs SET neohive_status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(run_id)
            .execute(pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary::workflows::models::{NeoHiveExportConfig, WorkflowRunStatus};
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool pinned to ONE connection so migrations + queries share the
    /// same database (a multi-connection :memory: pool loses the schema).
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn sample_input(name: &str) -> WorkflowInput {
        WorkflowInput {
            id: None,
            name: name.to_string(),
            description: Some("desc".to_string()),
            template_id: "standard_meeting".to_string(),
            custom_prompt: None,
            provider: "openrouter".to_string(),
            model: "anthropic/claude-sonnet-4".to_string(),
            max_tokens: Some(4096),
            temperature: Some(0.4),
            top_p: None,
            neohive_export: Some(NeoHiveExportConfig::default()),
        }
    }

    #[tokio::test]
    async fn create_then_list_returns_workflow() {
        let pool = test_pool().await;
        let created = WorkflowsRepository::upsert_workflow(&pool, &sample_input("Exec")).await.unwrap();
        assert!(!created.id.is_empty());
        let all = WorkflowsRepository::list_workflows(&pool).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Exec");
        assert_eq!(all[0].provider, "openrouter");
    }

    #[tokio::test]
    async fn upsert_with_id_updates_in_place() {
        let pool = test_pool().await;
        let created = WorkflowsRepository::upsert_workflow(&pool, &sample_input("Old")).await.unwrap();
        let mut update = sample_input("New");
        update.id = Some(created.id.clone());
        let updated = WorkflowsRepository::upsert_workflow(&pool, &update).await.unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "New");
        assert_eq!(WorkflowsRepository::list_workflows(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_removes_workflow_but_run_lifecycle_persists() {
        let pool = test_pool().await;
        // meetings row for the run FK
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        let wf = WorkflowsRepository::upsert_workflow(&pool, &sample_input("W")).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", Some(&wf.id), "W", "m1").await.unwrap();
        WorkflowsRepository::complete_run(&pool, "r1", "# Title\n**Summary**\nhi", "[]", WorkflowRunStatus::COMPLETED).await.unwrap();

        assert!(WorkflowsRepository::delete_workflow(&pool, &wf.id).await.unwrap());
        // run still retrievable (retained artifact)
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.status, "completed");
        assert_eq!(run.workflow_name, "W");
    }

    #[tokio::test]
    async fn run_lifecycle_create_complete_fail_status() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1','T','t','t')")
            .execute(&pool).await.unwrap();
        WorkflowsRepository::create_run(&pool, "r1", None, "Ad hoc", "m1").await.unwrap();
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.status, "queued");

        WorkflowsRepository::fail_run(&pool, "r1", "boom", WorkflowRunStatus::ERROR).await.unwrap();
        let run = WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap();
        assert_eq!(run.status, "error");
        assert_eq!(run.error.as_deref(), Some("boom"));

        WorkflowsRepository::set_run_neohive_status(&pool, "r1", "pushed").await.unwrap();
        assert_eq!(WorkflowsRepository::get_run(&pool, "r1").await.unwrap().unwrap().neohive_status, "pushed");

        let for_meeting = WorkflowsRepository::list_runs_for_meeting(&pool, "m1").await.unwrap();
        assert_eq!(for_meeting.len(), 1);
    }
}
