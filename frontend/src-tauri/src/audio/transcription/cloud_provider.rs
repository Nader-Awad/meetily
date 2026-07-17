use super::provider::{TranscriptionError, TranscriptionProvider, TranscriptResult};
use async_trait::async_trait;
use reqwest::Client;

/// Base URL for a named cloud STT preset. `custom` returns None (caller supplies it).
pub fn preset_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "groq" => Some("https://api.groq.com/openai/v1"),
        "openai" => Some("https://api.openai.com/v1"),
        _ => None,
    }
}

/// Encode 16 kHz mono f32 samples to a 16-bit PCM WAV byte buffer.
pub fn pcm16_wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits: u16 = 16;
    let byte_rate: u32 = sample_rate * channels as u32 * (bits as u32 / 8);
    let block_align: u16 = channels * (bits / 8);
    let data_len: u32 = (samples.len() * 2) as u32;
    let file_size: u32 = 36 + data_len;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav
}

/// Extract transcript text from an OpenAI-compatible /audio/transcriptions JSON body.
pub fn parse_transcription_text(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid transcription response JSON: {e}"))?;
    v.get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("Transcription response missing 'text': {body}"))
}

pub struct CloudProvider {
    base_url: String,
    api_key: String,
    model: String,
    client: Client,
}

impl CloudProvider {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self { base_url: base_url.trim_end_matches('/').to_string(), api_key, model, client: Client::new() }
    }
}

#[async_trait]
impl TranscriptionProvider for CloudProvider {
    async fn transcribe(&self, audio: Vec<f32>, language: Option<String>)
        -> std::result::Result<TranscriptResult, TranscriptionError> {
        if self.api_key.trim().is_empty() {
            return Err(TranscriptionError::EngineFailed("Missing API key for cloud transcription".into()));
        }
        let wav = pcm16_wav_bytes(&audio, 16000);
        let part = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| TranscriptionError::EngineFailed(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", self.model.clone());
        if let Some(lang) = language.filter(|l| !l.is_empty() && l != "auto" && l != "auto-translate") {
            form = form.text("language", lang);
        }
        let url = format!("{}/audio/transcriptions", self.base_url);
        let resp = self.client.post(&url).bearer_auth(&self.api_key).multipart(form).send().await
            .map_err(|e| TranscriptionError::EngineFailed(format!("Cloud STT request failed: {e}")))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| TranscriptionError::EngineFailed(e.to_string()))?;
        if !status.is_success() {
            return Err(TranscriptionError::EngineFailed(format!("Cloud STT {status}: {body}")));
        }
        let text = parse_transcription_text(&body).map_err(TranscriptionError::EngineFailed)?;
        Ok(TranscriptResult { text, confidence: None, is_partial: false })
    }
    async fn is_model_loaded(&self) -> bool { true }
    async fn get_current_model(&self) -> Option<String> { Some(self.model.clone()) }
    fn provider_name(&self) -> &'static str { "Cloud" }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_header_and_length() {
        let s = vec![0.0f32, 1.0, -1.0, 0.5];
        let w = pcm16_wav_bytes(&s, 16000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(&w[36..40], b"data");
        assert_eq!(w.len(), 44 + s.len() * 2); // header + 2 bytes/sample
        // clamping: 1.0 -> 32767, -1.0 -> -32767
        let i0 = i16::from_le_bytes([w[44], w[45]]);
        assert_eq!(i0, 0);
        let i1 = i16::from_le_bytes([w[46], w[47]]);
        assert_eq!(i1, 32767);
    }
    #[test]
    fn parse_text_ok_and_errors() {
        assert_eq!(parse_transcription_text(r#"{"text":"  hi "}"#).unwrap(), "hi");
        assert!(parse_transcription_text(r#"{"nope":1}"#).is_err());
        assert!(parse_transcription_text("not json").is_err());
    }
    #[test]
    fn presets_resolve() {
        assert_eq!(preset_base_url("groq"), Some("https://api.groq.com/openai/v1"));
        assert_eq!(preset_base_url("openrouter"), Some("https://openrouter.ai/api/v1"));
        assert_eq!(preset_base_url("openai"), Some("https://api.openai.com/v1"));
        assert_eq!(preset_base_url("custom"), None);
    }
}
