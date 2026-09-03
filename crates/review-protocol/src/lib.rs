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
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub schema: String,
    pub status: ReviewStatus,
    pub reason: ReviewReason,
    pub review_id: String,
    pub completed_at: String,
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
    let schemas = vec![
        ("review.request.v1.json", request_schema()),
        ("review.result.v1.json", review_schema()),
        ("agent.error.v1.json", error_schema()),
    ];
    schemas
}

fn request_schema() -> String {
    "{\n  \"$schema\":
 \"http://json-schema.org/draft/2020-12/schema\",\n
 \"additionalProperties\": false,\n  \"properties\": {\n
 \"base_ref\": {\n      \"type\": \"string\"\n    },\n    \"head_ref\":
 {\n      \"type\": \"string\"\n    },\n    \"repository_path\": {\n
 \"type\": \"string\"\n    },\n    \"schema\": {\n      \"type\":
 \"string\",\n      \"const\": \"review.request/v1\"\n    }\n  },\n
 \"required\": [\n    \"schema\", \"repository_path\", \"base_ref\",
 \"head_ref\"\n  ],\n  \"title\": \"Review Request\",\n  \"type\":
 \"object\"\n}\n"
        .to_string()
}

fn review_schema() -> String {
    "{\n  \"$schema\": \"http://json-schema.org/draft/2020-12/schema\",\n  \"title\": \"Review Result\",\n  \"type\": \"object\",\n  \"additionalProperties\": false,\n  \"required\": [\"schema\", \"status\", \"reason\", \"review_id\", \"completed_at\"],\n  \"properties\": {\n    \"schema\": {\n      \"type\": \"string\",\n      \"const\": \"review.result/v1\"\n    },\n    \"status\": {\n      \"type\": \"string\",\n      \"enum\": [\"APPROVED\", \"CHANGES_REQUESTED\", \"INDETERMINATE\"]\n    },\n    \"reason\": {\n      \"type\": \"string\",\n      \"enum\": [\"REVIEW_ENGINE_NOT_AVAILABLE\"]\n    },\n    \"review_id\": {\n      \"type\": \"string\"\n    },\n    \"completed_at\": {\n      \"type\": \"string\"\n    }\n  }\n}\n"
        .to_string()
}

fn error_schema() -> String {
    "{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n  \"title\": \"Agent Error\",\n  \"type\": \"object\",\n  \"additionalProperties\": false,\n  \"required\": [\"schema_version\", \"code\", \"category\", \"message\", \"retryable\"],\n  \"properties\": {\n    \"schema_version\": { \"type\": \"string\", \"const\": \"agent.error/v1\" },\n    \"code\": { \"type\": \"string\" },\n    \"category\": { \"type\": \"string\" },\n    \"message\": { \"type\": \"string\" },\n    \"retryable\": { \"type\": \"boolean\" }\n  }\n}\n"
        .to_string()
}
