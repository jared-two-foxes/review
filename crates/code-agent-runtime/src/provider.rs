use agent_kernel::model::{
    CanonicalModelRequest, CanonicalModelResponse, ModelAction, ModelError, ModelProvider,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// OpenAI-compatible model provider adapter.
///
/// Serializes canonical model requests into the OpenAI chat completion API
/// format and parses responses back into canonical model actions. All
/// provider-specific wire types stay private to this module.
pub struct OpenAiProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl OpenAiProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    fn http_post(&self, body: &str) -> Result<String, String> {
        let url = self
            .base_url
            .strip_prefix("http://")
            .ok_or("URL must start with http://")?;
        let (host_port, path) = url.split_once('/').ok_or("URL must contain a path")?;
        let (host, port) = host_port
            .rsplit_once(':')
            .map(|(h, p)| (h, p.parse::<u16>().unwrap_or(80)))
            .unwrap_or((host_port, 80));
        let path = format!("/{path}");

        let mut stream = TcpStream::connect((host, port)).map_err(|e| format!("connect: {e}"))?;
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();

        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            path,
            host,
            port,
            self.api_key,
            body.len(),
            body
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("write: {e}"))?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|e| format!("read: {e}"))?;
        let response_str = String::from_utf8_lossy(&response);
        let body_start = response_str
            .find("\r\n\r\n")
            .ok_or("no header/body separator in response")?;
        Ok(response_str[body_start + 4..].to_string())
    }
}

impl ModelProvider for OpenAiProvider {
    fn generate(
        &mut self,
        request: &CanonicalModelRequest,
    ) -> Result<CanonicalModelResponse, ModelError> {
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

        let response_body = match self.http_post(&body_str) {
            Ok(body) => body,
            Err(e) => {
                return Err(ModelError::Network(e));
            }
        };

        let response: Value = match serde_json::from_str(&response_body) {
            Ok(v) => v,
            Err(e) => {
                return Err(ModelError::ApiError(e.to_string()));
            }
        };

        let actions = response["choices"]
            .get(0)
            .and_then(|choice| choice["message"]["tool_calls"].as_array())
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| {
                        let id = call["id"].as_str()?;
                        let name = call["function"]["name"].as_str()?;
                        let args_str = call["function"]["arguments"].as_str()?;
                        let args: Value = serde_json::from_str(args_str).ok()?;
                        Some(ModelAction::ToolCall {
                            action_id: id.to_string(),
                            tool: name.to_string(),
                            arguments: args,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(CanonicalModelResponse { actions })
    }
}
