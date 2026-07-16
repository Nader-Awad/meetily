use log::info;
use tauri::{AppHandle, Emitter, Manager};

use super::manager::DatabaseManager;
use crate::state::AppState;

/// Initialize database on app startup
/// Handles first launch detection and conditional initialization
pub async fn initialize_database_on_startup(app: &AppHandle) -> Result<(), String> {
    // Check if this is the first launch (no database exists yet)
    let is_first_launch = DatabaseManager::is_first_launch(app)
        .await
        .map_err(|e| format!("Failed to check first launch status: {}", e))?;

    if is_first_launch {
        info!("First launch detected - will notify window when ready");

        // Delay event emission to ensure window is ready and React listeners are registered
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            app_handle
                .emit("first-launch-detected", ())
                .expect("Failed to emit first-launch-detected event");
            info!("Emitted first-launch-detected after delay");
        });
    } else {
        // Normal flow - initialize database immediately
        let db_manager = DatabaseManager::new_from_app_handle(app)
            .await
            .map_err(|e| format!("Failed to initialize database manager: {}", e))?;

        // Hydrate the custom-vocabulary hot-path global from the DB now that the pool
        // exists and migrations have run. Without this, a fresh app process that goes
        // straight to a batch path (audio import / retranscribe) instead of live
        // recording would never populate VOCABULARY_CONFIG, silently no-op'ing Whisper
        // term-biasing (Layer 2). Use the pool directly rather than app.state::<AppState>()
        // since this runs inside `setup()`, synchronously, before any command is invokable.
        let vocab_cfg = crate::database::repositories::setting::SettingsRepository::get_vocabulary_config(db_manager.pool())
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        crate::set_vocabulary_config_internal(vocab_cfg);

        app.manage(AppState { db_manager });
        info!("Database initialized successfully");
    }

    Ok(())
}
