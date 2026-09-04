use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
    UsageRecord,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// OpenAI-compatible model provider adapter.
///
/// Serializes canonical model requests into the OpenAI chat completion API
/// format and parses responses back into canonical model actions. All
/// provider-specific wire types stay private to this module.
pub struct OpenAiProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
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
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
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
                if let Ok(payload @ Value::Object(_)) = serde_json::from_str::<Value>(content) {
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
