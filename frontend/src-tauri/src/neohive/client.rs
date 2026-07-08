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

/// Interprets a JSON-RPC tools/call response: Err on a top-level protocol
/// error OR a tool-level `result.isError == true` (message pulled from
/// result.content text). Ok(()) only on a genuine success.
pub(crate) fn interpret_tool_response(parsed: &serde_json::Value) -> Result<(), String> {
    if let Some(err) = parsed.get("error") {
        return Err(format!("NeoHive memory_store error: {}", err));
    }
    if let Some(result) = parsed.get("result") {
        let is_error = result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_error {
            let msg = result.get("content")
                .and_then(|c| c.as_array())
                .map(|items| items.iter()
                    .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>().join("; "))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "tool reported isError with no content".to_string());
            return Err(format!("NeoHive memory_store failed: {}", msg));
        }
    }
    Ok(())
}

impl NeoHiveClient {
    pub fn new(endpoint: String, access_client_id: String, access_client_secret: String) -> Self {
        Self {
            endpoint,
            access_client_id,
            access_client_secret,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
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
            let snippet: String = text.chars().take(300).collect();
            return Err(format!("NeoHive initialize returned HTTP {} (check the Cloudflare Access service token and endpoint): {}", status, snippet));
        }
        // Surfaces auth/protocol errors early.
        let parsed = Self::parse_body(&ct, &text)?;
        if let Some(err) = parsed.get("error") {
            return Err(format!("NeoHive initialize error: {}", err));
        }
        if let Some(sid) = &session {
            *self.session_id.lock().await = Some(sid.clone());
        }
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let mut note_builder = self.base_request();
        if let Some(sid) = session {
            note_builder = note_builder.header("mcp-session-id", sid);
        }
        let _ = note_builder.json(&note).send().await;
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
        let sid_opt = self.session_id.lock().await.clone();
        if let Some(sid) = sid_opt { builder = builder.header("mcp-session-id", sid); }
        let resp = builder.json(&req_body).send().await
            .map_err(|e| format!("NeoHive memory_store request failed: {}", e))?;
        let status = resp.status();
        let ct = resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            let snippet: String = text.chars().take(300).collect();
            return Err(format!("NeoHive returned HTTP {}: {}", status, snippet));
        }
        let parsed = Self::parse_body(&ct, &text)?;
        interpret_tool_response(&parsed)?;
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

    #[test]
    fn interpret_tool_response_ok_on_success() {
        let v = serde_json::json!({"result":{"content":[{"type":"text","text":"stored"}],"isError":false}});
        assert!(interpret_tool_response(&v).is_ok());
    }

    #[test]
    fn interpret_tool_response_err_on_tool_iserror() {
        let v = serde_json::json!({"result":{"content":[{"type":"text","text":"bad type"}],"isError":true}});
        let e = interpret_tool_response(&v).unwrap_err();
        assert!(e.contains("bad type"));
    }

    #[test]
    fn interpret_tool_response_err_on_protocol_error() {
        let v = serde_json::json!({"error":{"code":-32602,"message":"invalid"}});
        assert!(interpret_tool_response(&v).is_err());
    }
}
