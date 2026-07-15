// diarization/mod.rs
//
// Speaker identification (diarization) for the live transcription pipeline.
// Rust-native and fully local: WeSpeaker CAM++ ONNX embeddings (via ort, the
// same runtime Parakeet uses) + online cosine clustering. Adapted from the
// diarization slice of upstream PR #538 (author: rodrigopg), trimmed of the
// overlap/timeline machinery. See docs in each module.

pub mod batch;
pub mod clustering;
pub mod commands;
pub mod embedding;
pub mod fbank;
pub mod flagging;
pub mod models;
pub mod normalize;
pub mod segmenter;
pub mod session;

pub use segmenter::{run_segmenter, DiarTurn};
pub use session::DiarizationSession;
