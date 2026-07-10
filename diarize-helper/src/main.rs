use std::io::Read;
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use speakrs::{ExecutionMode, OwnedDiarizationPipeline};

#[derive(Serialize)]
struct Turn {
    start_ms: u64,
    end_ms: u64,
    speaker: String,
}

fn arg_value(flag: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(a) = args.next() {
        if a == flag {
            return args.next();
        }
    }
    None
}

fn read_f32_le(path: &str) -> Result<Vec<f32>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("open samples file {}", path))?
        .read_to_end(&mut bytes)
        .context("read samples file")?;
    if bytes.len() % 4 != 0 {
        return Err(anyhow!("samples file length {} not a multiple of 4", bytes.len()));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn run() -> Result<()> {
    let samples_path = arg_value("--samples").ok_or_else(|| anyhow!("missing --samples"))?;
    let models_dir = arg_value("--models-dir").ok_or_else(|| anyhow!("missing --models-dir"))?;
    // speakrs loads its models from SPEAKRS_MODELS_DIR (downloads there on first run via the `online` feature).
    std::env::set_var("SPEAKRS_MODELS_DIR", &models_dir);

    let samples = read_f32_le(&samples_path)?;
    let mut pipeline = OwnedDiarizationPipeline::from_pretrained(ExecutionMode::CoreMl)
        .map_err(|e| anyhow!("speakrs from_pretrained failed: {e}"))?;
    let result = pipeline
        .run(&samples)
        .map_err(|e| anyhow!("speakrs run failed: {e}"))?;

    let turns: Vec<Turn> = result
        .discrete_diarization
        .to_segments()
        .into_iter()
        .map(|s| Turn {
            start_ms: (s.start.max(0.0) * 1000.0) as u64,
            end_ms: (s.end.max(0.0) * 1000.0) as u64,
            speaker: s.speaker,
        })
        .collect();

    println!("{}", serde_json::to_string(&turns)?);
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("diarize-helper error: {e:#}");
        std::process::exit(1);
    }
}
