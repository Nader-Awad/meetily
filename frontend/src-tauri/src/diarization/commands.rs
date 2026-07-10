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

/// Read all cluster centroids from a meeting folder's speakers.json, as
/// `(label, centroid)` pairs. Shares the JSON shape with
/// `load_centroid_from_folder`. Returns an empty vec on any missing/unparseable
/// file rather than erroring - callers treat this path as best-effort.
fn load_all_centroids_from_folder(folder: &str) -> Vec<(String, Vec<f32>)> {
    let path = std::path::Path::new(folder).join("speakers.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(json) => json,
        Err(_) => return Vec::new(),
    };
    json.get("speakers")
        .and_then(|s| s.as_array())
        .map(|speakers| {
            speakers
                .iter()
                .filter_map(|s| {
                    let label = s.get("label")?.as_str()?.to_string();
                    let centroid: Vec<f32> = s
                        .get("centroid")?
                        .as_array()?
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect();
                    if centroid.is_empty() {
                        None
                    } else {
                        Some((label, centroid))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Cluster labels (from the meeting's speakers.json) whose centroid matches the
/// newly-named speaker's centroid at/above `threshold` — candidates to merge
/// into the new name.
pub(crate) fn clusters_to_reattribute(
    named_centroid: &[f32],
    others: &[(String, Vec<f32>)],
    threshold: f32,
) -> Vec<String> {
    others
        .iter()
        .filter(|(_, c)| super::clustering::cosine_similarity(named_centroid, c) >= threshold)
        .map(|(label, _)| label.clone())
        .collect()
}

/// Best-effort: re-check the meeting's other cluster centroids against the
/// newly-named speaker's centroid and merge (relabel) the ones that match, in
/// both `transcripts` and `speakers.json`. Cleans up the case where one real
/// person was split across multiple "Speaker N" clusters. Any load/parse
/// failure or missing centroid silently skips re-attribution - the primary
/// rename this follows has already succeeded and must not be affected.
async fn reattribute_matching_clusters(
    pool: &SqlitePool,
    meeting_id: &str,
    folder_path: &str,
    new_name: &str,
    named_label: &str,
) {
    let all_centroids = load_all_centroids_from_folder(folder_path);
    if all_centroids.is_empty() {
        log::debug!(
            "🎙️ No speakers.json centroids for meeting {} - skipping re-attribution",
            meeting_id
        );
        return;
    }

    let named_centroid = match all_centroids.iter().find(|(label, _)| label == named_label) {
        Some((_, centroid)) => centroid.clone(),
        None => {
            log::debug!(
                "🎙️ No centroid found for '{}' in meeting {} - skipping re-attribution",
                named_label,
                meeting_id
            );
            return;
        }
    };

    let others: Vec<(String, Vec<f32>)> = all_centroids
        .iter()
        .filter(|(label, _)| label != named_label)
        .cloned()
        .collect();

    let hits = clusters_to_reattribute(&named_centroid, &others, super::clustering::PROFILE_MATCH_THRESHOLD);
    if hits.is_empty() {
        return;
    }

    let mut merged = 0usize;
    for hit in &hits {
        match sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
            .bind(new_name)
            .bind(meeting_id)
            .bind(hit)
            .execute(pool)
            .await
        {
            Ok(_) => merged += 1,
            Err(e) => log::warn!(
                "🎙️ Failed to re-attribute cluster '{}' in meeting {}: {}",
                hit,
                meeting_id,
                e
            ),
        }
    }

    if merged > 0 {
        let relabeled: Vec<(String, Vec<f32>)> = all_centroids
            .into_iter()
            .map(|(label, centroid)| {
                if label == named_label || hits.contains(&label) {
                    (new_name.to_string(), centroid)
                } else {
                    (label, centroid)
                }
            })
            .collect();
        let json = serde_json::json!({
            "version": "1.0",
            "speakers": relabeled.into_iter().map(|(label, centroid)| {
                serde_json::json!({ "label": label, "centroid": centroid })
            }).collect::<Vec<_>>(),
        });
        let path = std::path::Path::new(folder_path).join("speakers.json");
        match serde_json::to_string(&json).map(|s| std::fs::write(&path, s)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => log::warn!("🎙️ Failed to write speakers.json after re-attribution: {}", e),
            Err(e) => log::warn!("🎙️ Failed to serialize speakers.json after re-attribution: {}", e),
        }
    }

    log::info!(
        "🎙️ Re-attributed {} cluster(s) to '{}' in meeting {}",
        merged,
        new_name,
        meeting_id
    );
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

    // Best-effort: also merge any other clusters in this meeting whose voice
    // matches the speaker just named. Failures here (missing/unparseable
    // speakers.json, no centroid for old_label) are logged and skipped - the
    // rename above has already succeeded and must not be affected.
    let folder_path: Option<String> = sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(&meeting_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if let Some(folder) = folder_path.as_deref() {
        reattribute_matching_clusters(pool, &meeting_id, folder, new_name, &old_label).await;
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

#[cfg(test)]
mod reattribution_tests {
    use super::*;
    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }
    #[test]
    fn selects_only_matching_clusters() {
        let named = unit(vec![1.0, 0.05, 0.0]);
        let others = vec![
            ("Speaker 2".to_string(), unit(vec![0.97, 0.12, 0.0])), // match
            ("Speaker 3".to_string(), unit(vec![0.0, 1.0, 0.0])),   // no match
        ];
        let hits = clusters_to_reattribute(&named, &others, 0.6);
        assert_eq!(hits, vec!["Speaker 2".to_string()]);
    }
    #[test]
    fn empty_when_none_match() {
        let named = unit(vec![1.0, 0.0, 0.0]);
        let others = vec![("Speaker 2".to_string(), unit(vec![0.0, 1.0, 0.0]))];
        assert!(clusters_to_reattribute(&named, &others, 0.6).is_empty());
    }
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
