use review_app::ReviewConfig;
use review_protocol::{ReviewRequest, ReviewStatus};
use std::path::Path;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut request_path: Option<String> = None;
    let mut model: String = "gpt-4o".into();
    let mut base_url: String = "https://api.openai.com/v1/chat/completions".into();
    let mut max_turns: u32 = 10;
    let mut wall_clock_budget_secs: u64 = 60;
    let mut emit_events = false;
    let mut repository: Option<String> = None;
    let mut base_ref: Option<String> = None;
    let mut head_ref: Option<String> = None;
    let mut requirements_path: Option<String> = None;

    let mut i = 1; // Skip program name
    while i < args.len() {
        match args[i].as_str() {
            "run" => { /* subcommand marker, no-op for V0 */ }
            "--request" => {
                request_path = Some(flag_value(&args, i, "--request"));
                i += 1;
            }
            "--format" => {
                // V0 only supports JSON; accept and ignore for now.
                let value = flag_value(&args, i, "--format");
                if value != "json" {
                    emit_error(
                        "UNSUPPORTED_FORMAT",
                        &format!("unsupported format: {} (only json is supported)", value),
                    );
                }
                i += 1;
            }
            "--model" => {
                model = flag_value(&args, i, "--model");
                i += 1;
            }
            "--base-url" => {
                base_url = flag_value(&args, i, "--base-url");
                i += 1;
            }
            "--max-turns" => {
                max_turns = flag_value(&args, i, "--max-turns").parse().unwrap_or(10);
                i += 1;
            }
            "--wall-clock-budget-secs" => {
                wall_clock_budget_secs = flag_value(&args, i, "--wall-clock-budget-secs")
                    .parse()
                    .unwrap_or(60);
                i += 1;
            }
            "--emit-events" => {
                emit_events = true;
            }
            "--repository" => {
                repository = Some(flag_value(&args, i, "--repository"));
                i += 1;
            }
            "--base-ref" => {
                base_ref = Some(flag_value(&args, i, "--base-ref"));
                i += 1;
            }
            "--head-ref" => {
                head_ref = Some(flag_value(&args, i, "--head-ref"));
                i += 1;
            }
            "--requirements" => {
                requirements_path = Some(flag_value(&args, i, "--requirements"));
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error"))
    } else if emit_events {
        EnvFilter::new("info")
    } else {
        EnvFilter::new("error")
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let request: ReviewRequest = if let Some(path) = request_path {
        // File flow: the request (including optional requirements) comes from the file.
        let bytes = match cli_common::read_request(Path::new(&path)) {
            Ok(b) => b,
            Err(e) => emit_error("REQUEST_READ_FAILED", &e.to_string()),
        };
        match cli_common::parse_strict_json(&bytes) {
            Ok(r) => r,
            Err(cli_common::ParseError::Syntax(_e)) => {
                emit_error("REQUEST_PARSE_FAILED", "request is not valid JSON");
            }
            Err(cli_common::ParseError::DuplicateKey(key)) => {
                emit_error("DUPLICATE_KEY", &format!("duplicate key: {}", key));
            }
        }
    } else {
        // Demo shape: construct the request from CLI args
        let repository = match repository {
            Some(v) => v,
            None => {
                emit_error(
                    "MISSING_REPOSITORY",
                    "--repository or --request is required",
                );
            }
        };
        let base_ref = match base_ref {
            Some(v) => v,
            None => {
                emit_error("MISSING_BASE_REF", "--base-ref or --request is required");
            }
        };
        let head_ref = match head_ref {
            Some(v) => v,
            None => {
                emit_error("MISSING_HEAD_REF", "--head-ref or --request is required");
            }
        };
        ReviewRequest {
            schema: "review.request/v1".into(),
            repository_path: repository,
            base_ref,
            head_ref,
            requirements: requirements_path,
        }
    };

    if let Err(msg) = review_protocol::validate_request(&request) {
        emit_error("REQUEST_VALIDATION_FAILED", &msg);
    }

    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    let config = ReviewConfig {
        model,
        base_url,
        api_key,
        max_turns,
        max_tool_calls: 40,
        max_completion_attempts: 3,
        wall_clock_budget: Some(Duration::from_secs(wall_clock_budget_secs)),
    };
    let (result, events, setup_err) = review_app::run_review(&request, &config);

    if let Some(reason) = setup_err {
        tracing::warn!(error = %reason, "review setup failed");
    }

    cli_common::write_json_stdout(&result).expect("failed to write result");

    let exit = match result.status {
        ReviewStatus::Approved => cli_common::ExitCode::Approved,
        ReviewStatus::ChangesRequested => cli_common::ExitCode::ChangesRequested,
        ReviewStatus::Indeterminate => cli_common::ExitCode::Indeterminate,
    };

    std::process::exit(exit as i32);
}

fn emit_error(code: &str, message: &str) -> ! {
    let error = review_protocol::AgentError {
        schema_version: "agent.error/v1".to_string(),
        code: code.to_string(),
        category: "validation".to_string(),
        message: message.to_string(),
        retryable: false,
    };
    let _ = cli_common::write_json_stdout(&error);
    std::process::exit(cli_common::ExitCode::InvalidRequest as i32);
}

fn flag_value(args: &[String], index: usize, flag: &str) -> String {
    match args.get(index + 1) {
        Some(value) => value.clone(),
        None => {
            emit_error(
                "MISSING_ARGUMENT_VALUE",
                &format!("{} requires a value", flag),
            );
        }
    }
}
