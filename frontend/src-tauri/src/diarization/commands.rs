// diarization/commands.rs
//
// Tauri command surface for speaker identification: feature toggle
// (persisted in the diarization_settings table), model status, and
// model download.

use crate::database::repositories::speaker_profile::SpeakerProfilesRepository;
use crate::state::AppState;
use sqlx::SqlitePool;
use tauri::{command, AppHandle, Manager, Runtime};

pub async fn is_enabled(pool: &SqlitePool) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT enabled FROM diarization_settings WHERE id = '1'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|v| v != 0)
        .unwrap_or(false)
}

#[command]
pub async fn diarization_get_status<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let enabled = is_enabled(state.db_manager.pool()).await;
    let model_present = super::models::is_embedding_model_present(&app);
    Ok(serde_json::json!({
        "enabled": enabled,
        "model_present": model_present,
        "model_filename": super::models::EMBEDDING_MODEL_FILENAME,
    }))
}

#[command]
pub async fn diarization_set_enabled(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    sqlx::query(
        r#"
        INSERT INTO diarization_settings (id, enabled) VALUES ('1', $1)
        ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled
        "#,
    )
    .bind(enabled as i64)
    .execute(state.db_manager.pool())
    .await
    .map_err(|e| format!("Failed to save diarization setting: {}", e))?;
    log::info!("Speaker identification {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

#[command]
pub async fn diarization_download_model<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    super::models::download_embedding_model(&app).await
}

/// Read the centroid for a speaker label from a meeting folder's speakers.json.
fn load_centroid_from_folder(folder: &str, label: &str) -> Option<Vec<f32>> {
    let path = std::path::Path::new(folder).join("speakers.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("speakers")?.as_array()?.iter().find_map(|s| {
        if s.get("label")?.as_str()? != label {
            return None;
        }
        let centroid: Vec<f32> = s
            .get("centroid")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();
        if centroid.is_empty() {
            None
        } else {
            Some(centroid)
        }
    })
}

/// Rename a speaker across all segments of a meeting. Optionally saves the
/// speaker's voice centroid (from the meeting's speakers.json) as a persistent
/// profile so future recordings label this voice by name automatically.
#[command]
pub async fn diarization_rename_speaker(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    old_label: String,
    new_name: String,
    save_profile: bool,
) -> Result<serde_json::Value, String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("Speaker name cannot be empty".to_string());
    }
    let pool = state.db_manager.pool();

    let result = sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
        .bind(new_name)
        .bind(&meeting_id)
        .bind(&old_label)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to rename speaker: {}", e))?;
    let updated = result.rows_affected();

    let mut profile_saved = false;
    if save_profile {
        let folder_path: Option<String> =
            sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
                .bind(&meeting_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("Failed to look up meeting folder: {}", e))?
                .flatten();

        if let Some(centroid) = folder_path
            .as_deref()
            .and_then(|f| load_centroid_from_folder(f, &old_label))
        {
            SpeakerProfilesRepository::create(pool, new_name, &centroid)
                .await
                .map_err(|e| format!("Failed to save voice profile: {}", e))?;
            profile_saved = true;
            log::info!("Saved voice profile '{}' from meeting {}", new_name, meeting_id);
        } else {
            log::warn!(
                "No voice centroid found for '{}' in meeting {} - profile not saved",
                old_label,
                meeting_id
            );
        }
    }

    Ok(serde_json::json!({
        "updated_segments": updated,
        "profile_saved": profile_saved,
    }))
}

#[command]
pub async fn diarization_list_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let profiles = SpeakerProfilesRepository::list(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to list voice profiles: {}", e))?;
    Ok(profiles
        .into_iter()
        .map(|p| serde_json::json!({ "id": p.id, "name": p.name }))
        .collect())
}

#[command]
pub async fn diarization_rename_profile(
    state: tauri::State<'_, AppState>,
    id: String,
    name: String,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Profile name cannot be empty".to_string());
    }
    SpeakerProfilesRepository::rename(state.db_manager.pool(), &id, name)
        .await
        .map_err(|e| format!("Failed to rename voice profile: {}", e))
}

#[command]
pub async fn diarization_delete_profile(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    SpeakerProfilesRepository::delete(state.db_manager.pool(), &id)
        .await
        .map_err(|e| format!("Failed to delete voice profile: {}", e))
}

/// Create a diarization session for a recording/batch job when the feature is
/// enabled AND the embedding model is present. Seeds saved voice profiles so
/// returning speakers are labeled by name. Any failure returns None so speaker
/// labels are simply absent — transcription is never affected.
pub async fn init_session<R: Runtime>(
    app: &AppHandle<R>,
) -> Option<super::DiarizationSession> {
    let enabled = match app.try_state::<AppState>() {
        Some(state) => is_enabled(state.db_manager.pool()).await,
        None => false,
    };
    if !enabled {
        log::info!("🎙️ Speaker identification disabled");
        return None;
    }
    if !super::models::is_embedding_model_present(app) {
        log::warn!("🎙️ Speaker identification enabled but embedding model not downloaded - labels disabled");
        return None;
    }
    let model_path = match super::models::embedding_model_path(app) {
        Ok(path) => path,
        Err(e) => {
            log::warn!("🎙️ Could not resolve diarization model path: {}", e);
            return None;
        }
    };

    let profiles: Vec<(String, Vec<f32>)> = match app.try_state::<AppState>() {
        Some(state) => match SpeakerProfilesRepository::list(state.db_manager.pool()).await {
            Ok(profiles) => profiles.into_iter().map(|p| (p.name, p.embedding)).collect(),
            Err(e) => {
                log::warn!("🎙️ Failed to load voice profiles, continuing without: {}", e);
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let profile_count = profiles.len();

    match super::DiarizationSession::with_profiles(&model_path, profiles) {
        Ok(session) => {
            log::info!(
                "🎙️ ✅ Speaker identification active ({} saved profile{})",
                profile_count,
                if profile_count == 1 { "" } else { "s" }
            );
            Some(session)
        }
        Err(e) => {
            log::warn!("🎙️ Failed to initialize speaker identification: {}", e);
            None
        }
    }
}

/// Persist a session's speaker centroids to speakers.json in the meeting folder
/// so a later rename can save the voice as a persistent profile.
pub async fn persist_speaker_centroids(
    session: &super::DiarizationSession,
    folder: Option<std::path::PathBuf>,
) {
    persist_labeled_centroids(folder, &session.centroid_snapshot()).await;
}

/// Persist an explicit set of (label, centroid, unit count) speaker centroids
/// to speakers.json in the meeting folder. Used by the turn-based (sidecar)
/// batch diarization path, which maps local speakers to final profile-backed
/// labels itself rather than through a `DiarizationSession`'s online
/// clusterer. Same JSON shape as `persist_speaker_centroids` so rename /
/// "remember voice" (`load_centroid_from_folder`) reads it identically.
pub async fn persist_labeled_centroids(
    folder: Option<std::path::PathBuf>,
    centroids: &[(String, Vec<f32>, usize)],
) {
    if centroids.is_empty() {
        return;
    }
    let folder = match folder {
        Some(folder) => folder,
        None => {
            log::warn!("🎙️ No meeting folder available - speaker centroids not persisted");
            return;
        }
    };
    let json = serde_json::json!({
        "version": "1.0",
        "speakers": centroids.iter().map(|(label, centroid, count)| {
            serde_json::json!({ "label": label, "centroid": centroid, "segments": count })
        }).collect::<Vec<_>>(),
    });
    let path = folder.join("speakers.json");
    match serde_json::to_string(&json).map(|s| std::fs::write(&path, s)) {
        Ok(Ok(())) => log::info!("🎙️ Saved {} speaker centroid(s) to {}", centroids.len(), path.display()),
        Ok(Err(e)) => log::warn!("🎙️ Failed to write speakers.json: {}", e),
        Err(e) => log::warn!("🎙️ Failed to serialize speaker centroids: {}", e),
    }
}
