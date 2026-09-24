use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ConversationMessage, ModelAction, ModelError,
    ModelProvider, UsageRecord,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStyle {
    ChatCompletions,
    Responses,
}

pub enum AuthError {
    ExchangeFailed(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::ExchangeFailed(msg) => write!(f, "Token exchange failed: {}", msg),
        }
    }
}

trait RequestAuth: Send + Sync {
    /// Amend a request builder with teh authorization headers (and any
    /// provider-specific headers) required for this provider/credential
    /// pair to succeed.  May perform side effects such as token exchange
    /// or refresh, mutating internal state to cache the result.
    fn amend(
        &mut self,
        req: reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::RequestBuilder, AuthError>;
}

struct BearerAuth {
    token: String,
}

impl RequestAuth for BearerAuth {
    fn amend(
        &mut self,
        req: reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::RequestBuilder, AuthError> {
        Ok(req.header("Authorization", format!("Bearer {}", self.token)))
    }
}

struct CopilotAuth {
    github_token: String,
    cached_token: Option<CopilotToken>,
    client: reqwest::blocking::Client,
}

struct CopilotToken {
    token: String,
    expires_at: Instant,
}

impl CopilotAuth {
    fn new(github_token: String, client: reqwest::blocking::Client) -> Self {
        Self {
            github_token,
            cached_token: None,
            client,
        }
    }

    fn ensure_valid_token(&mut self) -> Result<&str, AuthError> {
        let needs_refresh = match &self.cached_token {
            Some(cached) => cached.expires_at <= Instant::now(),
            None => true,
        };

        if needs_refresh {
            // Exchange (or re-exchange) the GitHub token for a Copilot token.
            let resp = self
                .client
                .get("https://auth.github.com/copilot_internal/v2/token")
                .header("Authorization", format!("Token {}", self.github_token))
                .header("User-Agent", "GitHubCopilotChat/0.35.0")
                .header("Editor-Version", "vscode/1.107.0")
                .send()
                .map_err(|e| AuthError::ExchangeFailed(e.to_string()))?;

            let body: Value = resp
                .json()
                .map_err(|e| AuthError::ExchangeFailed(e.to_string()))?;
            let token = body["token"]
                .as_str()
                .ok_or(AuthError::ExchangeFailed("Missing token".into()))?;
            let expires_in = body["expires_in"]
                .as_u64()
                .ok_or(AuthError::ExchangeFailed("Missing expires_in".into()))?;
            self.cached_token = Some(CopilotToken {
                token: token.to_string(),
                expires_at: Instant::now() + Duration::from_secs(expires_in),
            });
        }

        Ok(&self.cached_token.as_ref().unwrap().token)
    }
}

impl RequestAuth for CopilotAuth {
    fn amend(
        &mut self,
        req: reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::RequestBuilder, AuthError> {
        let token = self.ensure_valid_token()?;
        Ok(req
            .header("Authorization", format!("Bearer {}", token))
            .header("Copilot-Integration-Id", "vscode-chat")
            .header("Editor-Version", "vscode/1.107.0")
            .header("Editor-Plugin-Version", "copilot-chat/0.35.0")
            .header("User-Agent", "GitHubCopilotChat/0.35.0"))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRoute {
    pub model: String,
    pub provider_root: String,
    pub provider: String,
    pub api_key: String,
    pub api_style: ApiStyle,
}

fn auth_for(
    provider: &str,
    api_key: String,
    client: reqwest::blocking::Client,
) -> Box<dyn RequestAuth> {
    match provider {
        "openai" | "ollama" | "opencode" => Box::new(BearerAuth { token: api_key }),
        "copilot" | "github-copilot" => {
            if api_key.starts_with("tid=") {
                Box::new(BearerAuth { token: api_key })
            } else {
                Box::new(CopilotAuth::new(api_key, client))
            }
        }
        _ => Box::new(BearerAuth { token: api_key }), // default
    }
}

fn resolve_api_style(provider: &str, model: &str) -> ApiStyle {
    match provider {
        // OpenAI and Copilot proxy the same OpenAI models with the same
        // endpoint requirements: gpt-5+, o-series, and codex models require
        // /responses; older models (gpt-4o, gpt-4.1) and non-OpenAI models
        // (Claude, Gemini) use /chat/completions
        "openai" | "copilot" | "github-copilot" => {
            if model.starts_with("gpt-5")
                || model.starts_with("gpt-6")
                || model.starts_with("o1")
                || model.starts_with("o3")
                || model.starts_with("o4")
                || model.contains("codex")
            {
                ApiStyle::Responses
            } else {
                ApiStyle::ChatCompletions
            }
        }
        "opencode" => {
            if model.starts_with("gpt-5")
                || model.starts_with("gpt-6")
                || model.starts_with("grok")
                || model.starts_with("mus-spark")
            {
                ApiStyle::Responses
            } else {
                ApiStyle::ChatCompletions
            }
        }
        "ollama" => ApiStyle::ChatCompletions,
        _ => ApiStyle::ChatCompletions, // default
    }
}

pub fn resolve_provider_route(
    model: &str,
    explicit_api_key: Option<&str>,
) -> Result<ProviderRoute, String> {
    resolve_provider_route_with_root(model, explicit_api_key, None)
}

pub fn resolve_provider_route_with_root(
    model: &str,
    explicit_api_key: Option<&str>,
    explicit_provider_root: Option<&str>,
) -> Result<ProviderRoute, String> {
    let parsed = model
        .split_once('/')
        .map(|(p, m)| (p.to_ascii_lowercase(), m));

    if let Some((provider, provider_model)) = parsed {
        if provider_model.is_empty() {
            return Err(format!(
                "model provider prefix '{}' requires a non-empty model name",
                provider
            ));
        }
        let (default_base_url, default_api_key) = match provider.as_str() {
            "openai" => (
                "https://api.openai.com/v1",
                std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            ),
            "ollama" => (
                "http://127.0.0.1:11434/v1",
                std::env::var("OLLAMA_API_KEY").unwrap_or_else(|_| "ollama".into()),
            ),
            "opencode" => (
                "https://opencode.ai/zen/v1",
                std::env::var("OPENCODE_API_KEY").unwrap_or_default(),
            ),
            "copilot" | "github-copilot" => (
                "https://api.githubcopilot.com",
                std::env::var("COPILOT_API_TOKEN")
                    .ok()
                    .filter(|t| !t.is_empty())
                    .or_else(|| {
                        std::env::var("GITHUB_TOKEN")
                            .or_else(|_| std::env::var("GITHUB_COPILOT_API_KEY"))
                            .ok()
                    })
                    .unwrap_or_default(),
            ),
            _ => {
                return Err(format!(
                    "unsupported model provider prefix '{}'; supported prefixes are openai/, ollama/, opencode/, copilot/, github-copilot/",
                    provider
                ));
            }
        };

        let api_style = resolve_api_style(provider.as_str(), provider_model);

        return Ok(ProviderRoute {
            model: provider_model.to_string(),
            provider_root: explicit_provider_root
                .unwrap_or(default_base_url)
                .trim_end_matches('/')
                .to_string(),
            provider: provider.to_string(),
            api_key: explicit_api_key
                .unwrap_or(default_api_key.as_str())
                .to_string(),
            api_style,
        });
    }

    if let Some(provider_root) = explicit_provider_root {
        return Ok(ProviderRoute {
            model: model.to_string(),
            provider_root: provider_root.trim_end_matches('/').to_string(),
            provider: String::new(),
            api_key: explicit_api_key
                .map(str::to_string)
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .unwrap_or_default(),
            api_style: ApiStyle::ChatCompletions,
        });
    }

    Err(format!(
        "model name '{}' does not contain a provider prefix; expected format is <provider>/<model>",
        model
    ))
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

trait WireFormat: Send + Sync {
    fn endpoint_suffix(&self) -> &'static str;
    fn serialize_request(&self, request: &CanonicalModelRequest, model: &str) -> Value;
    fn parse_response(&self, response: &Value) -> Vec<ModelAction>;
    fn normalize_usage(&self, response: &Value) -> UsageRecord;
}

struct ChatCompletionsWireFormat;

impl WireFormat for ChatCompletionsWireFormat {
    fn endpoint_suffix(&self) -> &'static str {
        "chat/completions"
    }

    fn serialize_request(&self, request: &CanonicalModelRequest, model: &str) -> Value {
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
                    },
                })
            })
            .collect();
        json!({
            "model": model,
            "messages": messages,
            "tools": tools,
        })
    }

    fn parse_response(&self, response: &Value) -> Vec<ModelAction> {
        let choice = response["choices"].get(0);
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
        }
    }

    fn normalize_usage(&self, response: &Value) -> UsageRecord {
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

struct ResponsesWireFormat;

impl WireFormat for ResponsesWireFormat {
    fn endpoint_suffix(&self) -> &'static str {
        "responses"
    }

    fn serialize_request(&self, request: &CanonicalModelRequest, model: &str) -> Value {
        // Instructions: top-level string, NOT a message in the input array.
        let instructions = request
            .instructions
            .iter()
            .map(|i| i.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let mut input: Vec<Value> = Vec::new();

        for context in &request.context {
            input.push(json!({
                "role": "user",
                "content": [{"type": "input_text", "text": context.content}],
            }));
        }

        for msg in &request.history {
            match msg {
                ConversationMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    if let Some(c) = content {
                        input.push(json!({"role": "assistant", "content": [{"type": "output_text", "text": c}]}));
                    }

                    for call in tool_calls {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": serde_json::to_string(&call.arguments).unwrap_or_default(),
                        }));
                    }
                }
                ConversationMessage::Tool {
                    tool_call_id,
                    content,
                } => {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_call_id,
                        "output": content
                    }));
                }
                ConversationMessage::User { content } => {
                    input.push(json!({"role": "user", "content": [{"type": "input_text", "text": content}]}));
                }
            }
        }
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect();

        json!({
            "model": model,
            "instructions": instructions,
            "input": input,
            "tools": tools,
        })
    }

    fn parse_response(&self, response: &Value) -> Vec<ModelAction> {
        let output = match response["output"].as_array() {
            Some(arr) => arr,
            None => return vec![],
        };

        let tool_calls: Vec<ModelAction> = output
            .iter()
            .filter(|item| item["type"].as_str() == Some("function_call"))
            .filter_map(|item| {
                let id = item["call_id"].as_str()?;
                let name = item["name"].as_str()?;
                let args: Value = match item["arguments"].clone() {
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
            .collect();

        if !tool_calls.is_empty() {
            return tool_calls;
        }

        let text: String = output
            .iter()
            .filter(|item| item["type"].as_str() == Some("message"))
            .filter_map(|item| item["content"].as_array())
            .flatten()
            .filter_map(|part| {
                if part["type"].as_str() == Some("output_text") {
                    part["text"].as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        if let Some(payload) = parse_content_completion(&text) {
            vec![ModelAction::CompletionRequest {
                action_id: "completion".into(),
                payload,
            }]
        } else {
            vec![]
        }
    }

    fn normalize_usage(&self, response: &Value) -> UsageRecord {
        let usage = &response["usage"];
        let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);
        let estimated_cost_usd =
            Some((input_tokens as f64 * 0.000001) + (output_tokens as f64 * 0.000002));
        UsageRecord {
            input_tokens,
            output_tokens,
            estimated_cost_usd,
        }
    }
}

/// OpenAI-compatible model provider adapter.
///
/// Serializes canonical model requests into the OpenAI chat completion API
/// format and parses responses back into canonical model actions. All
/// provider-specific wire types stay private to this module.
pub struct OpenAiProvider {
    pub provider_root: String,
    pub model: String,
    wire_format: Box<dyn WireFormat>,
    auth: Box<dyn RequestAuth>,
    pub trace_content: bool,
    client: reqwest::blocking::Client,
}

fn wire_format_for(style: ApiStyle) -> Box<dyn WireFormat> {
    match style {
        ApiStyle::ChatCompletions => Box::new(ChatCompletionsWireFormat),
        ApiStyle::Responses => Box::new(ResponsesWireFormat),
    }
}

impl OpenAiProvider {
    pub fn new(route: ProviderRoute) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        let wire_format = wire_format_for(route.api_style);
        let auth = auth_for(&route.provider, route.api_key, client.clone());
        let trace_content = std::env::var("REVIEW_TRACE_CONTENT")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        Self {
            provider_root: route.provider_root,
            auth,
            model: route.model,
            wire_format,
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

        let body = self.wire_format.serialize_request(request, &self.model);
        let body_str = serde_json::to_string(&body).unwrap_or_default();

        let resp = self.http_post(&body_str, per_request_timeout)?;

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

        let actions = self.wire_format.parse_response(&response);
        let usage = self.wire_format.normalize_usage(&response);

        Ok(CanonicalModelResponse {
            actions,
            usage: Some(usage),
        })
    }

    fn http_post(
        &mut self,
        body: &str,
        timeout: Option<Duration>,
    ) -> Result<reqwest::blocking::Response, ModelError> {
        let url = format!(
            "{}/{}",
            self.provider_root,
            self.wire_format.endpoint_suffix()
        );
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_string());
        req = self
            .auth
            .amend(req)
            .map_err(|e| ModelError::ApiError(e.to_string()))?;
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        req.send().map_err(|e| {
            if e.is_timeout() {
                ModelError::Timeout(e.to_string())
            } else {
                ModelError::ApiError(e.to_string())
            }
        })
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
