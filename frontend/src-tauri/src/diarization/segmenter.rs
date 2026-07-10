// diarization/segmenter.rs
//
// Calls the `diarize-helper` sidecar (speakrs pyannote diarization) as a
// subprocess: writes the decoded 16 kHz mono samples as raw f32 LE to a temp
// file, runs the bundled sidecar, and parses its JSON turns from stdout.
// Best-effort: any failure returns None so the batch path degrades to the
// existing VAD behavior — diarization never breaks transcription.
//
// This codebase has no tauri-plugin-shell (no `capabilities/` dir);
// `externalBin` in tauri.conf.json is only a bundling/validation mechanism.
// `resolve_sidecar_binary` below mirrors
// `summary::summary_engine::sidecar::SidecarManager::resolve_helper_binary`
// (same current_exe / RESOURCE_DIR / target-triple resolution), and the
// sidecar itself is spawned one-shot via plain `tokio::process::Command`
// rather than the persistent stdin/stdout loop llama-helper uses.

use serde::Deserialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, Runtime};

/// Monotonic counter mixed into the temp samples filename below. The pid
/// alone (`std::process::id()`) is constant for the whole process, so two
/// concurrent sidecar calls in the same run (e.g. a retranscribe racing an
/// import) would otherwise collide on the same temp path.
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Deserialize)]
pub struct DiarTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
}

pub(crate) fn parse_turns_json(stdout: &str) -> Option<Vec<DiarTurn>> {
    serde_json::from_str::<Vec<DiarTurn>>(stdout.trim()).ok()
}

/// Directory speakrs models live in (downloaded on first sidecar run).
fn speakrs_models_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()?
        .join("models")
        .join("diarization")
        .join("speakrs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Resolve the path to the bundled diarize-helper binary. Mirrors
/// `summary::summary_engine::sidecar::SidecarManager::resolve_helper_binary`,
/// substituting "diarize-helper" for "llama-helper". Returns `None` (not an
/// error) on any miss so callers degrade to no diarization instead of failing.
fn resolve_sidecar_binary() -> Option<PathBuf> {
    // 1. Check environment variable (dev mode or manual override)
    if let Ok(env_path) = std::env::var("MEETILY_DIARIZE_HELPER") {
        if !env_path.is_empty() {
            let path = PathBuf::from(env_path);
            if path.exists() {
                log::info!("🗣️ Using diarize-helper from MEETILY_DIARIZE_HELPER: {}", path.display());
                return Some(path);
            }
        }
    }

    let target_triple = || {
        std::env::var("TARGET").unwrap_or_else(|_| {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            { "x86_64-unknown-linux-gnu".to_string() }
            #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
            { "aarch64-unknown-linux-gnu".to_string() }
            #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
            { "x86_64-apple-darwin".to_string() }
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            { "aarch64-apple-darwin".to_string() }
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            { "x86_64-pc-windows-msvc".to_string() }
            #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
            { "aarch64-pc-windows-msvc".to_string() }
            #[cfg(not(any(
                all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")),
                all(target_os = "macos", any(target_arch = "x86_64", target_arch = "aarch64")),
                all(target_os = "windows", any(target_arch = "x86_64", target_arch = "aarch64"))
            )))]
            { "unknown".to_string() }
        })
    };

    // 2. Check relative to current executable (most reliable for bundled apps)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            log::info!("🗣️ Searching for diarize-helper relative to executable: {}", exe_dir.display());

            let binary_name = if cfg!(windows) {
                format!("diarize-helper-{}.exe", target_triple())
            } else {
                format!("diarize-helper-{}", target_triple())
            };

            let bundled = exe_dir.join(&binary_name);
            if bundled.exists() {
                log::info!("🗣️ Found exact match next to executable: {}", bundled.display());
                return Some(bundled);
            }

            if let Ok(entries) = std::fs::read_dir(exe_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("diarize-helper") && !name.ends_with(".d") {
                            log::info!("🗣️ Found fuzzy match next to executable: {}", path.display());
                            return Some(path);
                        }
                    }
                }
            }
        }
    }

    // 3. Check bundled resources (RESOURCE_DIR) — fallback
    if let Ok(resource_dir) = std::env::var("RESOURCE_DIR") {
        log::info!("🗣️ Searching for diarize-helper in RESOURCE_DIR: {}", resource_dir);
        let resource_path = PathBuf::from(&resource_dir);

        let binary_name = if cfg!(windows) {
            format!("diarize-helper-{}.exe", target_triple())
        } else {
            format!("diarize-helper-{}", target_triple())
        };

        let bundled = resource_path.join(&binary_name);
        if bundled.exists() {
            log::info!("🗣️ Found exact match in RESOURCE_DIR: {}", bundled.display());
            return Some(bundled);
        }

        if let Ok(entries) = std::fs::read_dir(&resource_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("diarize-helper") && !name.ends_with(".d") {
                        log::info!("🗣️ Found fuzzy match in RESOURCE_DIR: {}", path.display());
                        return Some(path);
                    }
                }
            }
        }
    } else {
        log::warn!("🗣️ RESOURCE_DIR environment variable not set");
    }

    // 4. Fallback for dev: try relative paths from workspace (no target triple in dev builds).
    // `diarize-helper` is its own nested workspace (has its own `[workspace]` in
    // diarize-helper/Cargo.toml), so it builds to
    // `<repo>/diarize-helper/target/{release,debug}/`, not `<repo>/target/...`.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let project_root = PathBuf::from(&manifest_dir).parent()?.parent()?.to_path_buf();

        let candidates = [
            project_root.join("diarize-helper/target/release/diarize-helper"),
            project_root.join("diarize-helper/target/debug/diarize-helper"),
            project_root.join("diarize-helper/target/release/diarize-helper.exe"),
            project_root.join("diarize-helper/target/debug/diarize-helper.exe"),
        ];

        for candidate in candidates {
            if candidate.exists() {
                log::info!("🗣️ Using dev diarize-helper: {}", candidate.display());
                return Some(candidate);
            }
        }
    }

    None
}

/// Run the diarize-helper sidecar over 16 kHz mono samples. None on any failure.
pub async fn run_segmenter<R: Runtime>(
    app: &AppHandle<R>,
    samples_16k: &[f32],
) -> Option<Vec<DiarTurn>> {
    if samples_16k.is_empty() {
        return None;
    }
    let models_dir = speakrs_models_dir(app)?;

    // Write raw f32 LE samples to a temp file. Mix a counter and a timestamp
    // into the name alongside the pid — the pid is constant per process, so
    // it alone can't disambiguate two concurrent calls (e.g. retranscribe +
    // import racing) within the same run.
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!(
        "meetily_diar_{}_{}_{}.f32",
        std::process::id(),
        nanos,
        counter
    ));
    {
        let mut f = match std::fs::File::create(&tmp) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("🗣️ diarization: could not create temp samples file: {}", e);
                return None;
            }
        };
        let mut buf = Vec::with_capacity(samples_16k.len() * 4);
        for s in samples_16k {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        if let Err(e) = f.write_all(&buf) {
            log::warn!("🗣️ diarization: could not write temp samples: {}", e);
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
    }

    // Resolve + spawn the bundled diarize-helper sidecar as a ONE-SHOT process.
    let bin = match resolve_sidecar_binary() {
        Some(b) => b,
        None => {
            log::warn!("🗣️ diarization: diarize-helper sidecar binary not found");
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
    };
    let result = tokio::process::Command::new(&bin)
        .arg("--samples")
        .arg(&tmp)
        .arg("--models-dir")
        .arg(&models_dir)
        .output()
        .await
        .ok();

    let _ = std::fs::remove_file(&tmp);

    let output = result?;
    if !output.status.success() {
        log::warn!(
            "🗣️ diarization sidecar exited non-zero: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let turns = parse_turns_json(&stdout);
    match &turns {
        Some(t) => log::info!("🗣️ diarization sidecar produced {} turn(s)", t.len()),
        None => log::warn!("🗣️ diarization sidecar output was not valid JSON"),
    }
    turns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_turns() {
        let json = r#"[{"start_ms":0,"end_ms":1500,"speaker":"SPEAKER_00"},
                       {"start_ms":1500,"end_ms":3000,"speaker":"SPEAKER_01"}]"#;
        let turns = parse_turns_json(json).expect("should parse");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].end_ms, 1500);
        assert_eq!(turns[1].speaker, "SPEAKER_01");
    }

    #[test]
    fn empty_array_is_some_empty() {
        assert_eq!(parse_turns_json("[]").map(|t| t.len()), Some(0));
    }

    #[test]
    fn malformed_is_none() {
        assert!(parse_turns_json("not json").is_none());
        assert!(parse_turns_json("").is_none());
    }
}
