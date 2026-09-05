// Protocol scaffolding for the end-to-end test.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum ReviewStatus {
    #[serde(rename = "INDETERMINATE")]
    Indeterminate,
    #[serde(rename = "APPROVED")]
    Approved,
    #[serde(rename = "CHANGES_REQUESTED")]
    ChangesRequested,
}

#[derive(Serialize, Deserialize)]
pub enum ReviewReason {
    #[serde(rename = "REVIEW_ENGINE_NOT_AVAILABLE")]
    ReviewEngineNotAvailable,
    #[serde(rename = "REVIEW_COMPLETED")]
    ReviewCompleted,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub schema: String,
    pub repository_path: String,
    pub base_ref: String,
    pub head_ref: String,
    #[serde(default)]
    pub requirements: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FindingOutput {
    pub blocking: bool,
    pub message: String,
    pub severity: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub recommendation: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct UsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub schema: String,
    pub status: ReviewStatus,
    pub reason: ReviewReason,
    pub review_id: String,
    pub completed_at: String,
    pub findings: Vec<FindingOutput>,
    pub usage: UsageSummary,
}

#[derive(Serialize, Deserialize)]
pub struct AgentError {
    pub schema_version: String,
    pub code: String,
    pub category: String,
    pub message: String,
    pub retryable: bool,
}

pub fn validate_request(request: &ReviewRequest) -> Result<(), String> {
    // Check schema version
    if request.schema != "review.request/v1" {
        return Err(format!("unsupported schema version: {}", request.schema));
    }
    // Check repository path exists
    if !std::path::Path::new(&request.repository_path).exists() {
        return Err(format!(
            "repository path does not exist: {}",
            request.repository_path
        ));
    }
    if request.base_ref.is_empty() {
        return Err("base_ref must not be empty".into());
    }
    if request.head_ref.is_empty() {
        return Err("head_ref must not be empty".into());
    }
    Ok(())
}

pub fn generate_schemas() -> Vec<(&'static str, String)> {
    vec![
        ("review.request.v1.json", request_schema()),
        ("review.result.v1.json", review_schema()),
        ("agent.error.v1.json", error_schema()),
    ]
}

fn request_schema() -> String {
    "{\n  \"$schema\": \"http://json-schema.org/draft/2020-12/schema\",\n  \"additionalProperties\": false,\n  \"properties\": {\n    \"base_ref\": {\n      \"type\": \"string\"\n    },\n    \"head_ref\": {\n      \"type\": \"string\"\n    },\n    \"repository_path\": {\n      \"type\": \"string\"\n    },\n    \"requirements\": {\n      \"type\": [\"string\", \"null\"]\n    },\n    \"schema\": {\n      \"type\": \"string\",\n      \"const\": \"review.request/v1\"\n    }\n  },\n  \"required\": [\n    \"schema\", \"repository_path\", \"base_ref\", \"head_ref\"\n  ],\n  \"title\": \"Review Request\",\n  \"type\": \"object\"\n}\n".to_string()
}

fn review_schema() -> String {
    r#"{
  "$schema": "http://json-schema.org/draft/2020-12/schema",
  "title": "Review Result",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema", "status", "reason", "review_id", "completed_at", "findings", "usage"],
  "properties": {
    "schema": {
      "type": "string",
      "const": "review.result/v1"
    },
    "status": {
      "type": "string",
      "enum": ["APPROVED", "CHANGES_REQUESTED", "INDETERMINATE"]
    },
    "reason": {
      "type": "string",
      "enum": ["REVIEW_ENGINE_NOT_AVAILABLE", "REVIEW_COMPLETED"]
    },
    "review_id": {
      "type": "string"
    },
    "completed_at": {
      "type": "string"
    },
    "findings": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["blocking", "message", "severity"],
        "properties": {
          "blocking": { "type": "boolean" },
          "message": { "type": "string" },
          "severity": { "type": "string" },
          "path": { "type": ["string", "null"] },
          "line": { "type": ["integer", "null"] },
          "recommendation": { "type": ["string", "null"] }
        }
      }
    },
    "usage": {
      "type": "object",
      "additionalProperties": false,
      "required": ["input_tokens", "output_tokens"],
      "properties": {
        "input_tokens": { "type": "integer" },
        "output_tokens": { "type": "integer" },
        "estimated_cost_usd": { "type": ["number", "null"] }
      }
    }
  }
}
"#
    .to_string()
}

fn error_schema() -> String {
    "{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n  \"title\": \"Agent Error\",\n  \"type\": \"object\",\n  \"additionalProperties\": false,\n  \"required\": [\"schema_version\", \"code\", \"category\", \"message\", \"retryable\"],\n  \"properties\": {\n    \"schema_version\": { \"type\": \"string\", \"const\": \"agent.error/v1\" },\n    \"code\": { \"type\": \"string\" },\n    \"category\": { \"type\": \"string\" },\n    \"message\": { \"type\": \"string\" },\n    \"retryable\": { \"type\": \"boolean\" }\n  }\n}\n"
        .to_string()
}
