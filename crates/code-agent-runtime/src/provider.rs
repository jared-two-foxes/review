use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ConversationMessage, ModelAction, ModelError,
    ModelProvider, UsageRecord,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tracing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRoute {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
}

pub fn resolve_provider_route(
    model: &str,
    explicit_base_url: Option<&str>,
    explicit_api_key: Option<&str>,
) -> Result<ProviderRoute, String> {
    let parsed = model
        .split_once('/')
        .map(|(p, m)| (p.to_ascii_lowercase(), m))
        .filter(|(_, m)| !m.is_empty());

    if let Some((provider, provider_model)) = parsed {
        let (default_base_url, default_api_key) = match provider.as_str() {
            "openai" => (
                "https://api.openai.com/v1/chat/completions",
                std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            ),
            "ollama" => (
                "http://127.0.0.1:11434/v1/chat/completions",
                std::env::var("OLLAMA_API_KEY").unwrap_or_else(|_| "ollama".into()),
            ),
            "opencode" => (
                "https://api.opencode.ai/v1/chat/completions",
                std::env::var("OPENCODE_API_KEY").unwrap_or_default(),
            ),
            "copilot" | "github-copilot" => (
                "https://api.githubcopilot.com/chat/completions",
                std::env::var("GITHUB_TOKEN")
                    .or_else(|_| std::env::var("GITHUB_COPILOT_API_KEY"))
                    .unwrap_or_default(),
            ),
            _ => {
                return Err(format!(
                    "unsupported model provider prefix '{}'; supported prefixes are openai/, ollama/, opencode/, copilot/, github-copilot/",
                    provider
                ));
            }
        };

        return Ok(ProviderRoute {
            model: provider_model.to_string(),
            base_url: explicit_base_url.unwrap_or(default_base_url).to_string(),
            api_key: explicit_api_key.unwrap_or(default_api_key.as_str()).to_string(),
        });
    }

    let default_base_url = "https://api.openai.com/v1/chat/completions";
    let default_api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    Ok(ProviderRoute {
        model: model.to_string(),
        base_url: explicit_base_url.unwrap_or(default_base_url).to_string(),
        api_key: explicit_api_key.unwrap_or(default_api_key.as_str()).to_string(),
    })
}

fn parse_content_completion(content: &str) -> Option<Value> {
    // 1. Pure JSON object (struct):
    if let Ok(obj @ Value::Object(_)) = serde_json::from_str::<Value>(content) {
        return Some(obj);
    }
    // 2. Markdown ```json block:
    if let Some(start) = content.find("```json") {
        let after = &content[start + "```json".len()..];
        if let Some(end) = after.find("```") {
            let json_str = after[..end].trim();
            if let Ok(obj @ Value::Object(_)) = serde_json::from_str::<Value>(json_str) {
                return Some(obj);
            }
        }
    }
    // 3. Generic ``` block (no language label):
    if let Some(start) = content.find("```") {
        let after = &content[start + 3..];
        // Skip a possible language label on first line.
        let body_start = after.find('\n').map(|n| n + 1).unwrap_or(0);
        let body = &after[body_start..];
        if let Some(end) = body.find("```") {
            let json_str = body[..end].trim();
            if let Ok(obj @ Value::Object(_)) = serde_json::from_str::<Value>(json_str) {
                return Some(obj);
            }
        }
    }
    // 4. Find a JSON object embedded in prose (base JSON at the end of text).
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find('{') {
        let abs_pos = search_from + pos;
        if let Ok(obj @ Value::Object(_)) = serde_json::from_str::<Value>(&content[abs_pos..]) {
            return Some(obj);
        }
        search_from = abs_pos + 1;
    }
    None
}

/// OpenAI-compatible model provider adapter.
///
/// Serializes canonical model requests into the OpenAI chat completion API
/// format and parses responses back into canonical model actions. All
/// provider-specific wire types stay private to this module.
pub struct OpenAiProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub trace_content: bool,
    client: reqwest::blocking::Client,
}

impl OpenAiProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        let trace_content = std::env::var("REVIEW_TRACE_CONTENT")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            trace_content,
            client,
        }
    }

    fn generate_impl(
        &mut self,
        request: &CanonicalModelRequest,
        deadline: Option<Instant>,
    ) -> Result<CanonicalModelResponse, ModelError> {
        let per_request_timeout = match deadline {
            Some(d) => match d.checked_duration_since(Instant::now()) {
                Some(r) if r > Duration::ZERO => Some(r),
                _ => {
                    return Err(ModelError::Timeout(
                        "caller deadline already exceeded".into(),
                    ));
                }
            },
            None => None, // fall back to the client's 30s default
        };

        let mut messages = Vec::new();
        for instruction in &request.instructions {
            messages.push(json!({
                "role": "system",
                "content": instruction.content,
            }));
        }
        for context in &request.context {
            messages.push(json!({
                "role": "user",
                "content": context.content,
            }));
        }
        for msg in &request.history {
            match msg {
                ConversationMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    let calls_json: Vec<Value> = tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": serde_json::to_string(&call.arguments).unwrap_or_default(),
                                }
                            })
                        })
                        .collect();
                    let mut entry = json!({"role":"assistant"});
                    if let Some(c) = content {
                        entry["content"] = json!(c);
                    }
                    if !calls_json.is_empty() {
                        entry["tool_calls"] = json!(calls_json);
                    }
                    messages.push(entry);
                }
                ConversationMessage::Tool {
                    tool_call_id,
                    content,
                } => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": content,
                    }));
                }
                ConversationMessage::User { content } => {
                    messages.push(json!({
                        "role": "user",
                        "content": content,
                    }));
                }
            }
        }
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect();
        let body = json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
        });
        let body_str = serde_json::to_string(&body).unwrap_or_default();

        let resp = self
            .http_post(&body_str, per_request_timeout)
            .map_err(|e| {
                if e.is_timeout() {
                    ModelError::Timeout(e.to_string())
                } else {
                    ModelError::Network(e.to_string())
                }
            })?;

        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(ModelError::RateLimit(format!(
                "Rate limit: {}",
                status.as_u16()
            )));
        } else if !status.is_success() {
            return Err(ModelError::ApiError(format!(
                "API error: {}",
                status.as_u16()
            )));
        }

        let response: Value = resp
            .json()
            .map_err(|e| ModelError::ApiError(e.to_string()))?;

        if let Some(choice) = response["choices"].get(0) {
            let finish_reason = choice["finish_reason"].as_str().unwrap_or("unknown");
            let tool_names: Vec<&str> = choice["message"]["tool_calls"]
                .as_array()
                .map(|calls| {
                    calls
                        .iter()
                        .filter_map(|c| c["function"]["name"].as_str())
                        .collect()
                })
                .unwrap_or_default();
            let content = choice["message"]["content"].as_str().unwrap_or("");
            tracing::debug!(
                finish_reason,
                tools = ?tool_names,
                content_present = !content.is_empty(),
                "model response"
            );
            if self.trace_content && !content.is_empty() {
                let preview = if content.len() > 300 {
                    format!("{}...", &content[..300])
                } else {
                    content.to_string()
                };
                tracing::trace!(content_preview = %preview, "OpenAI response content");
            }
        }

        let choice = response["choices"].get(0);
        let actions: Vec<ModelAction> =
            if let Some(calls) = choice.and_then(|c| c["message"]["tool_calls"].as_array()) {
                calls
                    .iter()
                    .filter_map(|call| {
                        let id = call["id"].as_str()?;
                        let name = call["function"]["name"].as_str()?;
                        let args: Value = match call["function"]["arguments"].clone() {
                            Value::String(s) if s.is_empty() => json!({}),
                            Value::String(s) => match serde_json::from_str(&s) {
                                Ok(parsed @ Value::Object(_)) => parsed,
                                _ => return None,
                            },
                            obj @ Value::Object(_) => obj,
                            _ => return None,
                        };
                        Some(ModelAction::ToolCall {
                            action_id: id.to_string(),
                            tool: name.to_string(),
                            arguments: args,
                        })
                    })
                    .collect()
            } else if let Some(content) = choice.and_then(|c| c["message"]["content"].as_str()) {
                // A live model returns its findings as message content (a JSON object).
                // Convert that into a completion request so the kernel can validate it.
                if let Some(payload) = parse_content_completion(content) {
                    vec![ModelAction::CompletionRequest {
                        action_id: "completion".into(),
                        payload,
                    }]
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

        let usage = Self::normalize_usage(&response);
        Ok(CanonicalModelResponse {
            actions,
            usage: Some(usage),
        })
    }

    fn http_post(
        &self,
        body: &str,
        timeout: Option<Duration>,
    ) -> Result<reqwest::blocking::Response, reqwest::Error> {
        let mut req = self
            .client
            .post(&self.base_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .body(body.to_string());
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        req.send()
    }

    /// Parse provider usage into the provider-independent accounting contract.
    pub fn normalize_usage(response: &Value) -> UsageRecord {
        let usage = &response["usage"];
        let input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0);
        let output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0);
        let estimated_cost_usd =
            Some((input_tokens as f64 * 0.000001) + (output_tokens as f64 * 0.000002));
        UsageRecord {
            input_tokens,
            output_tokens,
            estimated_cost_usd,
        }
    }
}

impl ModelProvider for OpenAiProvider {
    fn generate(
        &mut self,
        request: &CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse, ModelError> {
        self.generate_impl(request, None)
    }

    /// Execute a generation request with a caller-supplied deadline.
    ///
    /// Enforces the caller-supplied deadline via a per-request HTTP timeout.  Returns
    /// ModelError::Timeout if the provider has not responded by deadline.
    fn generate_with_deadline(
        &mut self,
        request: &CanonicalModelRequest,
        deadline: Instant,
    ) -> Result<CanonicalModelResponse, ModelError> {
        self.generate_impl(request, Some(deadline))
    }
}
