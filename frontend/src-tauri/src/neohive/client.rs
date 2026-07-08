use serde_json::{json, Value};

pub struct NeoHiveClient {
    endpoint: String,
    access_client_id: String,
    access_client_secret: String,
    http: reqwest::Client,
    session_id: tokio::sync::Mutex<Option<String>>,
}

/// Pure: the params for a tools/call memory_store request.
pub(crate) fn build_store_params(
    content: &str,
    mem_type: &str,
    tags: &[String],
    importance: u8,
) -> Value {
    json!({
        "name": "memory_store",
        "arguments": {
            "content": content,
            "type": mem_type,
            "tags": tags,
            "importance": importance
        }
    })
}

impl NeoHiveClient {
    pub fn new(endpoint: String, access_client_id: String, access_client_secret: String) -> Self {
        Self {
            endpoint,
            access_client_id,
            access_client_secret,
            http: reqwest::Client::new(),
            session_id: tokio::sync::Mutex::new(None),
        }
    }

    /// Base request with Cloudflare Access service-token headers + MCP Accept.
    fn base_request(&self) -> reqwest::RequestBuilder {
        self.http
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("CF-Access-Client-Id", &self.access_client_id)
            .header("CF-Access-Client-Secret", &self.access_client_secret)
            .header("x-mcp-client", "meetily")
    }

    /// Extract the JSON-RPC object from a plain-JSON or SSE body.
    fn parse_body(content_type: &str, body: &str) -> Result<Value, String> {
        if content_type.contains("text/event-stream") {
            for line in body.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    let rest = rest.trim();
                    if rest.starts_with('{') {
                        return serde_json::from_str(rest)
                            .map_err(|e| format!("NeoHive SSE parse error: {}", e));
                    }
                }
            }
            Err("NeoHive SSE response contained no data payload".to_string())
        } else {
            serde_json::from_str(body).map_err(|e| format!("NeoHive JSON parse error: {}", e))
        }
    }

    async fn ensure_session(&self) -> Result<(), String> {
        {
            let guard = self.session_id.lock().await;
            if guard.is_some() {
                return Ok(());
            }
        }
        let init = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "meetily", "version": "0.4.0" }
            }
        });
        let resp = self.base_request().json(&init).send().await
            .map_err(|e| format!("NeoHive initialize failed: {}", e))?;
        let status = resp.status();
        let session = resp.headers().get("mcp-session-id")
            .and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        let ct = resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("NeoHive initialize returned HTTP {} (check the Cloudflare Access service token and endpoint)", status));
        }
        // Surfaces auth/protocol errors early.
        let parsed = Self::parse_body(&ct, &text)?;
        if let Some(err) = parsed.get("error") {
            return Err(format!("NeoHive initialize error: {}", err));
        }
        if let Some(sid) = session {
            *self.session_id.lock().await = Some(sid.clone());
            let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
            let _ = self.base_request().header("mcp-session-id", sid).json(&note).send().await;
        }
        Ok(())
    }

    pub async fn store_memory(
        &self,
        content: &str,
        mem_type: &str,
        tags: &[String],
        importance: u8,
    ) -> Result<(), String> {
        self.ensure_session().await?;
        let params = build_store_params(content, mem_type, tags, importance);
        let req_body = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": params });

        let mut builder = self.base_request();
        if let Some(sid) = self.session_id.lock().await.clone() {
            builder = builder.header("mcp-session-id", sid);
        }
        let resp = builder.json(&req_body).send().await
            .map_err(|e| format!("NeoHive memory_store request failed: {}", e))?;
        let status = resp.status();
        let ct = resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("NeoHive returned HTTP {}", status));
        }
        let parsed = Self::parse_body(&ct, &text)?;
        if let Some(err) = parsed.get("error") {
            return Err(format!("NeoHive memory_store error: {}", err));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_store_params_matches_memory_store_contract() {
        let p = build_store_params("hello", "insight", &["a".into(), "b".into()], 6);
        assert_eq!(p["name"], "memory_store");
        assert_eq!(p["arguments"]["content"], "hello");
        assert_eq!(p["arguments"]["type"], "insight");
        assert_eq!(p["arguments"]["importance"], 6);
        assert_eq!(p["arguments"]["tags"][0], "a");
    }
}
