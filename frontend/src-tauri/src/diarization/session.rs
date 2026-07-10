// diarization/session.rs
//
// Per-recording diarization state: embedding extractor + online clusterer.
// Created when a recording starts (if the feature is enabled and the model
// is present) and dropped when it ends. Adapted from upstream PR #538
// (author: rodrigopg), trimmed to the per-segment labeling path (the
// rolling-window timeline / overlap machinery is out of scope for v1).

use super::clustering::SpeakerClusterer;
use super::embedding::{EmbeddingError, EmbeddingExtractor};
use std::path::Path;

/// Minimum samples needed for the fbank frontend to produce the 10 frames
/// required by EmbeddingExtractor::compute (25ms frame + 9 * 10ms shifts).
const MIN_SAMPLES_FOR_EMBEDDING: usize = 1_840;

fn has_enough_samples_for_embedding(samples_len: usize) -> bool {
    samples_len >= MIN_SAMPLES_FOR_EMBEDDING
}

pub struct DiarizationSession {
    extractor: EmbeddingExtractor,
    clusterer: SpeakerClusterer,
}

impl DiarizationSession {
    pub fn new(embedding_model_path: &Path) -> Result<Self, EmbeddingError> {
        Self::with_profiles(embedding_model_path, Vec::new())
    }

    /// Create a session pre-seeded with saved voice profiles (name, centroid)
    /// so returning speakers are labeled by name instead of "Speaker N".
    pub fn with_profiles(
        embedding_model_path: &Path,
        profiles: Vec<(String, Vec<f32>)>,
    ) -> Result<Self, EmbeddingError> {
        let mut clusterer = SpeakerClusterer::new();
        for (name, centroid) in profiles {
            clusterer.seed_profile(&name, centroid);
        }
        Ok(Self {
            extractor: EmbeddingExtractor::new(embedding_model_path)?,
            clusterer,
        })
    }

    /// (label, centroid, segment count) snapshot for persisting this
    /// recording's speakers (written to speakers.json at recording end).
    pub fn centroid_snapshot(&self) -> Vec<(String, Vec<f32>, usize)> {
        self.clusterer
            .centroids()
            .map(|(label, centroid, count)| (label.to_string(), centroid.to_vec(), count))
            .collect()
    }

    /// Assign a speaker label to a 16kHz mono speech segment.
    /// Returns None only when no label can be produced (e.g. first segment
    /// is too short). Diarization failures must never break transcription —
    /// errors are logged and degrade to the previous label or None.
    pub fn label_segment(&mut self, samples_16k: &[f32]) -> Option<String> {
        if !has_enough_samples_for_embedding(samples_16k.len()) {
            return self.clusterer.last_label();
        }
        match self.extractor.compute(samples_16k) {
            Ok(embedding) => Some(self.clusterer.assign(&embedding)),
            Err(e) => {
                log::warn!(
                    "Diarization embedding failed, carrying previous label: {}",
                    e
                );
                self.clusterer.last_label()
            }
        }
    }

    /// Compute the raw L2-normalized CAM++ embedding for a 16kHz mono segment,
    /// without touching the clusterer. Used by the batch/retranscription turn
    /// path to build per-local-speaker centroids from the sidecar's turns
    /// (clustering there is done via profile matching in `batch::map_local_speakers_to_profiles`
    /// instead of the online clusterer). None if the segment is too short or
    /// extraction fails — callers should simply skip that unit's embedding.
    pub fn embed(&mut self, samples_16k: &[f32]) -> Option<Vec<f32>> {
        if !has_enough_samples_for_embedding(samples_16k.len()) {
            return None;
        }
        match self.extractor.compute(samples_16k) {
            Ok(embedding) => Some(embedding),
            Err(e) => {
                log::warn!("Diarization embedding failed for batch unit: {}", e);
                None
            }
        }
    }

    pub fn clusterer(&self) -> &SpeakerClusterer {
        &self.clusterer
    }

    pub fn clusterer_mut(&mut self) -> &mut SpeakerClusterer {
        &mut self.clusterer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_gate_matches_minimum_fbank_frames() {
        assert!(!has_enough_samples_for_embedding(
            MIN_SAMPLES_FOR_EMBEDDING - 1
        ));
        assert!(has_enough_samples_for_embedding(MIN_SAMPLES_FOR_EMBEDDING));
    }
}
