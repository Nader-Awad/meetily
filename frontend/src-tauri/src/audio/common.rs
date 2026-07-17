use crate::api::TranscriptSegment;
use anyhow::Result;
use log::{debug, info};
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

static ENGINE_LIFECYCLE_LOCK: Lazy<Arc<AsyncMutex<()>>> =
    Lazy::new(|| Arc::new(AsyncMutex::new(())));

pub(crate) async fn acquire_engine_lifecycle_lock() -> OwnedMutexGuard<()> {
    ENGINE_LIFECYCLE_LOCK.clone().lock_owned().await
}

/// Unload the transcription engine after a batch job (import or retranscription).
/// Skips unloading if a live recording is currently in progress, since recording
/// uses the same global engine instances. Cloud providers hold no local model,
/// so there is nothing to unload for them.
pub(crate) async fn unload_engine_after_batch(provider: &str) {
    let _engine_lifecycle_guard = acquire_engine_lifecycle_lock().await;

    if crate::audio::recording_commands::is_recording().await {
        log::info!("Skipping model unload after batch: recording in progress");
        return;
    }

    match provider {
        // Cloud providers keep no local model in memory — nothing to unload.
        "openrouter" | "groq" | "openai" | "custom" => {
            log::debug!(
                "No local transcription engine to unload for cloud provider '{}'",
                provider
            );
        }
        "parakeet" => {
            use crate::parakeet_engine::commands::PARAKEET_ENGINE;
            let engine = {
                let guard = PARAKEET_ENGINE.lock().unwrap_or_else(|e| e.into_inner());
                guard.as_ref().cloned()
            };
            if let Some(e) = engine {
                e.unload_model().await;
            }
        }
        // localWhisper / whisper / anything else defaults to the Whisper engine,
        // matching the pre-refactor `!use_parakeet` behavior.
        _ => {
            use crate::whisper_engine::commands::WHISPER_ENGINE;
            let engine = {
                let guard = WHISPER_ENGINE.lock().unwrap_or_else(|e| e.into_inner());
                guard.as_ref().cloned()
            };
            if let Some(e) = engine {
                e.unload_model().await;
            }
        }
    }
}

/// Build a cloud transcription provider (`Arc<dyn TranscriptionProvider>`) for a
/// batch job (import or retranscription) from stored settings: API key, base URL,
/// and model. Shared by both batch selectors for the
/// `openrouter` / `groq` / `openai` / `custom` providers so the multipart cloud
/// request path is identical to the live engine's `CloudProvider`.
pub(crate) async fn build_cloud_provider<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    provider: &str,
    requested_model: Option<&str>,
) -> std::result::Result<Arc<dyn crate::audio::transcription::TranscriptionProvider>, String> {
    use crate::database::repositories::setting::SettingsRepository;
    use tauri::Manager;

    let app_state = app
        .try_state::<crate::state::AppState>()
        .ok_or_else(|| "App state not available".to_string())?;
    let pool = app_state.db_manager.pool();

    // Load the stored transcript settings once — used for the model fallback and
    // the custom base URL.
    let stored = SettingsRepository::get_transcript_config(pool)
        .await
        .map_err(|e| format!("Failed to load transcript config: {}", e))?;

    // API key is required for any cloud provider.
    let api_key = SettingsRepository::get_transcript_api_key(pool, provider)
        .await
        .map_err(|e| format!("Failed to load API key for provider '{}': {}", provider, e))?
        .unwrap_or_default();
    if api_key.trim().is_empty() {
        return Err(format!(
            "Cloud transcription provider '{}' requires an API key (set it in Settings → Transcription).",
            provider
        ));
    }

    // Model: an explicitly requested model wins; otherwise fall back to the
    // stored config model.
    let model = match requested_model {
        Some(m) if !m.trim().is_empty() => m.to_string(),
        _ => stored.as_ref().map(|s| s.model.clone()).unwrap_or_default(),
    };
    if model.trim().is_empty() {
        return Err(
            "Cloud transcription requires a model (e.g. openai/whisper-large-v3).".to_string(),
        );
    }

    // Base URL: a named preset resolves by provider; `custom` uses the stored URL.
    let base_url = crate::audio::transcription::cloud_provider::preset_base_url(provider)
        .map(|s| s.to_string())
        .or_else(|| stored.as_ref().and_then(|s| s.transcript_base_url.clone()))
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| format!("No base URL configured for provider '{}'.", provider))?;

    info!(
        "☁️ Using cloud transcription provider '{}' (model {}) for batch job",
        provider, model
    );

    Ok(Arc::new(
        crate::audio::transcription::cloud_provider::CloudProvider::new(base_url, api_key, model),
    ))
}

/// Create transcript segments from transcription results.
/// Each tuple is (text, start_ms, end_ms, speaker) from VAD timestamps + optional diarization label.
pub(crate) fn create_transcript_segments(
    transcripts: &[(String, f64, f64, Option<String>)],
    corrections: &[crate::vocabulary::Correction],
) -> Vec<TranscriptSegment> {
    transcripts
        .iter()
        .map(|(text, start_ms, end_ms, speaker)| {
            let start_seconds = start_ms / 1000.0;
            let end_seconds = end_ms / 1000.0;
            let duration = end_seconds - start_seconds;

            TranscriptSegment {
                id: format!("transcript-{}", Uuid::new_v4()),
                text: crate::vocabulary::apply_corrections(text.trim(), corrections),
                timestamp: chrono::Utc::now().to_rfc3339(),
                audio_start_time: Some(start_seconds),
                audio_end_time: Some(end_seconds),
                duration: Some(duration),
                speaker: speaker.clone(),
            }
        })
        .collect()
}

/// Write transcripts.json to a meeting folder (atomic write with temp file)
pub(crate) fn write_transcripts_json(folder: &Path, segments: &[TranscriptSegment]) -> Result<()> {
    let transcript_path = folder.join("transcripts.json");
    let temp_path = folder.join(".transcripts.json.tmp");

    let json = serde_json::json!({
        "version": "1.0",
        "last_updated": chrono::Utc::now().to_rfc3339(),
        "total_segments": segments.len(),
        "segments": segments.iter().enumerate().map(|(i, s)| {
            serde_json::json!({
                "id": s.id,
                "text": s.text,
                "timestamp": s.timestamp,
                "audio_start_time": s.audio_start_time,
                "audio_end_time": s.audio_end_time,
                "duration": s.duration,
                "sequence_id": i
            })
        }).collect::<Vec<_>>()
    });

    let json_string = serde_json::to_string_pretty(&json)?;
    std::fs::write(&temp_path, &json_string)?;
    std::fs::rename(&temp_path, &transcript_path)?;

    info!(
        "Wrote transcripts.json with {} segments to {}",
        segments.len(),
        transcript_path.display()
    );
    Ok(())
}

/// Split a long speech segment at the lowest-energy (silence) point near the target size.
///
/// Scans for 100ms windows with minimal RMS energy within +/-3 seconds of each target
/// split point. If no clear silence is found, falls back to a 1-second overlap split
/// to avoid cutting words at boundaries.
pub(crate) fn split_segment_at_silence(
    segment: &crate::audio::vad::SpeechSegment,
    max_samples: usize,
) -> Vec<crate::audio::vad::SpeechSegment> {
    const SAMPLE_RATE: usize = 16000;
    // 100ms window for energy measurement (1600 samples at 16kHz)
    const ENERGY_WINDOW: usize = SAMPLE_RATE / 10;
    // Search +/-3 seconds around the target split point
    const SEARCH_RADIUS: usize = SAMPLE_RATE * 3;
    // RMS threshold below which we consider a window "silent"
    const SILENCE_RMS_THRESHOLD: f32 = 0.02;
    // Overlap to use when no silence boundary is found (1 second)
    const FALLBACK_OVERLAP: usize = SAMPLE_RATE;

    let total = segment.samples.len();
    if total <= max_samples {
        return vec![segment.clone()];
    }

    let ms_per_sample = (segment.end_timestamp_ms - segment.start_timestamp_ms)
        / segment.samples.len() as f64;
    let mut result = Vec::new();
    let mut pos = 0usize;

    while pos < total {
        let remaining = total - pos;
        if remaining <= max_samples {
            // Last chunk - take everything remaining
            let chunk_samples = segment.samples[pos..].to_vec();
            let chunk_start_ms = segment.start_timestamp_ms + (pos as f64 * ms_per_sample);
            let chunk_end_ms = segment.end_timestamp_ms;
            result.push(crate::audio::vad::SpeechSegment {
                samples: chunk_samples,
                start_timestamp_ms: chunk_start_ms,
                end_timestamp_ms: chunk_end_ms,
                confidence: segment.confidence,
            });
            break;
        }

        // Target split point
        let target = pos + max_samples;

        // Search window: [target - SEARCH_RADIUS, target + SEARCH_RADIUS]
        let search_start = target.saturating_sub(SEARCH_RADIUS).max(pos + SAMPLE_RATE);
        let search_end = (target + SEARCH_RADIUS).min(total.saturating_sub(ENERGY_WINDOW));

        // Find the lowest-energy 100ms window in the search range
        let mut best_split = target.min(total); // fallback: exact target
        let mut best_rms = f32::MAX;

        if search_start + ENERGY_WINDOW <= search_end {
            let mut idx = search_start;
            while idx + ENERGY_WINDOW <= search_end {
                let window = &segment.samples[idx..idx + ENERGY_WINDOW];
                let rms = (window.iter().map(|s| s * s).sum::<f32>() / ENERGY_WINDOW as f32).sqrt();
                if rms < best_rms {
                    best_rms = rms;
                    best_split = idx + ENERGY_WINDOW / 2; // split at center of quiet window
                }
                // Step by 10ms (160 samples) for efficiency
                idx += SAMPLE_RATE / 100;
            }
        }

        let split_at = best_split;
        if best_rms <= SILENCE_RMS_THRESHOLD {
            debug!(
                "Splitting at silence boundary: sample {} (RMS={:.4})",
                split_at, best_rms
            );
        } else {
            debug!(
                "No silence found near target (best RMS={:.4}), splitting with overlap at sample {}",
                best_rms, split_at
            );
        }

        // Determine the actual end of this chunk (with overlap if no silence)
        let chunk_end = if best_rms > SILENCE_RMS_THRESHOLD {
            (split_at + FALLBACK_OVERLAP).min(total)
        } else {
            split_at
        };

        let chunk_samples = segment.samples[pos..chunk_end].to_vec();
        let chunk_start_ms = segment.start_timestamp_ms + (pos as f64 * ms_per_sample);
        let chunk_end_ms = segment.start_timestamp_ms + (chunk_end as f64 * ms_per_sample);

        result.push(crate::audio::vad::SpeechSegment {
            samples: chunk_samples,
            start_timestamp_ms: chunk_start_ms,
            end_timestamp_ms: chunk_end_ms,
            confidence: segment.confidence,
        });

        // Advance position to where the current chunk actually ends
        // to avoid transcribing the overlap region twice
        pos = chunk_end;
    }

    result
}

/// Split a diarization turn-unit's samples into transcribable sub-pieces.
/// Units at or under `max_samples` come back as a single sub-piece with the
/// unit's original bounds (via `split_segment_at_silence`'s own short-circuit);
/// longer ones — a multi-minute single-speaker monologue — are split at
/// silence boundaries so they don't become one giant transcription call.
/// Callers should still embed the full `unit_samples` once for the speaker
/// centroid *before* calling this, since it consumes the buffer; only the
/// transcription unit changes here, not the speaker attribution.
pub(crate) fn split_unit_for_transcription(
    unit_samples: Vec<f32>,
    start_ms: u64,
    end_ms: u64,
    max_samples: usize,
) -> Vec<crate::audio::vad::SpeechSegment> {
    let segment = crate::audio::vad::SpeechSegment {
        samples: unit_samples,
        start_timestamp_ms: start_ms as f64,
        end_timestamp_ms: end_ms as f64,
        confidence: 1.0,
    };
    split_segment_at_silence(&segment, max_samples)
}

/// Slice 16 kHz mono samples for a [start_ms, end_ms) window (clamped).
/// Used by the diarization turn path to cut a single-speaker unit out of the
/// full decoded buffer for per-unit transcription/embedding.
pub(crate) fn slice_samples_16k(samples: &[f32], start_ms: u64, end_ms: u64) -> Vec<f32> {
    let sr = 16_000u64;
    let start = ((start_ms * sr) / 1000) as usize;
    let end = (((end_ms * sr) / 1000) as usize).min(samples.len());
    if start >= end {
        return Vec::new();
    }
    samples[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_engine_lifecycle_lock_serializes_acquirers() {
        let guard = acquire_engine_lifecycle_lock().await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (acquired_tx, mut acquired_rx) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async {
            started_tx.send(()).unwrap();
            let _guard = acquire_engine_lifecycle_lock().await;
            acquired_tx.send(()).unwrap();
        });

        started_rx.await.unwrap();
        assert!(acquired_rx.try_recv().is_err());
        drop(guard);

        acquired_rx.await.unwrap();
        waiter.await.unwrap();
    }
}
