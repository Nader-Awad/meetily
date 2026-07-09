use serde_json::{json, Value};

/// Authentication for the NeoHive MCP endpoint. Built from a stored
/// (type string, fields JSON) pair via `from_parts` and applied to each request.
#[derive(Debug, Clone)]
pub enum NeoHiveAuth {
    CloudflareAccess { client_id: String, client_secret: String },
    Bearer { token: String },
    Basic { username: String, password: String },
    CustomHeader { header_name: String, header_value: String },
    None,
}

impl NeoHiveAuth {
    /// Map the snake_case type + camelCase fields JSON into an auth method.
    /// Blank/missing required fields for a known type, or an unknown type, error.
    pub fn from_parts(auth_type: Option<&str>, config: &serde_json::Value) -> Result<Self, String> {
        let req = |k: &str| -> Result<String, String> {
            config.get(k).and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .ok_or_else(|| format!("NeoHive auth: missing/empty field '{}'", k))
        };
        match auth_type {
            Some("cloudflare_access") => Ok(NeoHiveAuth::CloudflareAccess { client_id: req("clientId")?, client_secret: req("clientSecret")? }),
            Some("bearer") => Ok(NeoHiveAuth::Bearer { token: req("token")? }),
            Some("basic") => Ok(NeoHiveAuth::Basic { username: req("username")?, password: req("password")? }),
            Some("custom_header") => Ok(NeoHiveAuth::CustomHeader { header_name: req("headerName")?, header_value: req("headerValue")? }),
            Some("none") | None => Ok(NeoHiveAuth::None),
            Some(other) => Err(format!("NeoHive auth: unknown type '{}'", other)),
        }
    }

    /// Attach this method's auth to a request builder. Invariant headers are set by the caller.
    fn apply(&self, b: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            NeoHiveAuth::CloudflareAccess { client_id, client_secret } => b
                .header("CF-Access-Client-Id", client_id)
                .header("CF-Access-Client-Secret", client_secret),
            NeoHiveAuth::Bearer { token } => b.bearer_auth(token),
            NeoHiveAuth::Basic { username, password } => b.basic_auth(username, Some(password)),
            NeoHiveAuth::CustomHeader { header_name, header_value } => b.header(header_name.as_str(), header_value.as_str()),
            NeoHiveAuth::None => b,
        }
    }
}

pub struct NeoHiveClient {
    endpoint: String,
    auth: NeoHiveAuth,
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
    pub fn new(endpoint: String, auth: NeoHiveAuth) -> Self {
        Self {
            endpoint,
            auth,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            session_id: tokio::sync::Mutex::new(None),
        }
    }

    /// Base request with the configured auth method's headers + MCP Accept.
    fn base_request(&self) -> reqwest::RequestBuilder {
        let b = self.http
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("x-mcp-client", "meetily");
        self.auth.apply(b)
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
            return Err(format!("NeoHive initialize returned HTTP {} (check the endpoint and the configured auth credentials): {}", status, snippet));
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

    fn header_val(req: &reqwest::Request, name: &str) -> Option<String> {
        req.headers().get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
    }

    #[test]
    fn apply_cloudflare_sets_cf_headers_and_no_auth() {
        let req = NeoHiveAuth::CloudflareAccess { client_id: "id1".into(), client_secret: "sec1".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert_eq!(header_val(&req, "cf-access-client-id").as_deref(), Some("id1"));
        assert_eq!(header_val(&req, "cf-access-client-secret").as_deref(), Some("sec1"));
        assert!(header_val(&req, "authorization").is_none());
    }

    #[test]
    fn apply_bearer_sets_authorization() {
        let req = NeoHiveAuth::Bearer { token: "tok".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert_eq!(header_val(&req, "authorization").as_deref(), Some("Bearer tok"));
    }

    #[test]
    fn apply_basic_sets_base64_authorization() {
        let req = NeoHiveAuth::Basic { username: "user".into(), password: "pass".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        // base64("user:pass") == "dXNlcjpwYXNz"
        assert_eq!(header_val(&req, "authorization").as_deref(), Some("Basic dXNlcjpwYXNz"));
    }

    #[test]
    fn apply_custom_header_sets_it() {
        let req = NeoHiveAuth::CustomHeader { header_name: "X-Api-Key".into(), header_value: "k".into() }
            .apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert_eq!(header_val(&req, "x-api-key").as_deref(), Some("k"));
    }

    #[test]
    fn apply_none_sets_no_auth_headers() {
        let req = NeoHiveAuth::None.apply(reqwest::Client::new().post("http://x")).build().unwrap();
        assert!(header_val(&req, "authorization").is_none());
        assert!(header_val(&req, "cf-access-client-id").is_none());
    }

    #[test]
    fn from_parts_maps_types_and_validates() {
        use serde_json::json;
        assert!(matches!(NeoHiveAuth::from_parts(Some("cloudflare_access"), &json!({"clientId":"a","clientSecret":"b"})).unwrap(), NeoHiveAuth::CloudflareAccess{..}));
        assert!(matches!(NeoHiveAuth::from_parts(Some("bearer"), &json!({"token":"t"})).unwrap(), NeoHiveAuth::Bearer{..}));
        assert!(matches!(NeoHiveAuth::from_parts(Some("none"), &json!({})).unwrap(), NeoHiveAuth::None));
        assert!(matches!(NeoHiveAuth::from_parts(None, &json!({})).unwrap(), NeoHiveAuth::None));
        assert!(NeoHiveAuth::from_parts(Some("bearer"), &json!({"token":""})).is_err());     // blank required field
        assert!(NeoHiveAuth::from_parts(Some("mystery"), &json!({})).is_err());               // unknown type
    }
}
