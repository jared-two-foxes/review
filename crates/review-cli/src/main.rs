use review_app::ReviewConfig;
use review_protocol::{ReviewRequest, ReviewStatus};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut request_path: Option<String> = None;
    let mut model: String = "gpt-4o".into();
    let mut base_url: Option<String> = None;
    let mut max_turns: u32 = 10;
    let mut wall_clock_budget_secs: u64 = 60;
    let mut max_input_tokens: Option<u64> = None;
    let mut max_cost_usd: Option<f64> = None;
    let mut emit_events = false;
    let mut repository: Option<String> = None;
    let mut base_ref: Option<String> = None;
    let mut head_ref: Option<String> = None;
    let mut requirements_path: Option<String> = None;
    let mut uncommitted = false;
    let mut ledger_path: Option<String> = None;

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
                base_url = Some(flag_value(&args, i, "--base-url"));
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
            "--max-input-tokens" => {
                max_input_tokens = flag_value(&args, i, "--max-input-tokens").parse().ok();
                i += 1;
            }
            "--max-cost-usd" => {
                max_cost_usd = flag_value(&args, i, "--max-cost-usd").parse().ok();
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
            "--uncommitted" => {
                uncommitted = true;
            }
            "--ledger" => {
                ledger_path = Some(flag_value(&args, i, "--ledger"));
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
        if uncommitted && head_ref.is_some() {
            emit_error(
                "INVALID_ARGUMENTS",
                "--uncommitted cannot be used with --head-ref",
            );
        }
        let base_ref = base_ref.unwrap_or_else(|| "HEAD".into());
        let head_ref = head_ref.unwrap_or_else(|| ":working".into());

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

    let cancellation_token = Arc::new(AtomicBool::new(false));
    let token_clone = Arc::clone(&cancellation_token);
    ctrlc::set_handler(move || {
        token_clone.store(true, Ordering::Relaxed);
    })
    .expect("set Ctrl-C handler");

    let route = resolve_provider_route(&model, base_url.as_deref());
    let config = ReviewConfig {
        model: route.model,
        base_url: route.base_url,
        api_key: route.api_key,
        max_turns,
        max_tool_calls: 40,
        max_completion_attempts: 3,
        wall_clock_budget: Some(Duration::from_secs(wall_clock_budget_secs)),
        max_input_tokens,
        max_cost_usd,
        ledger_path: ledger_path.map(|p| std::path::PathBuf::from(p)),
        max_repeated_actions: 3,
    };
    let (result, _events, setup_err) =
        review_app::run_review(&request, &config, Some(&cancellation_token));

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

    struct ProviderRoute {
        model: String,
        base_url: String,
        api_key: String,
    }

    fn resolve_provider_route(model: &str, explicit_base_url: Option<&str>) -> ProviderRoute {
        let (provider, provider_model) = model
            .split_once('/')
            .map(|(p, m)| (p.to_ascii_lowercase(), m))
            .filter(|(_, m)| !m.is_empty())
            .unwrap_or(("openai".into(), model));

        let (default_base_url, api_key) = match provider.as_str() {
            "ollama" => (
                "http://127.0.0.1:11434/v1/chat/completions",
                std::env::var("OLLAMA_API_KEY").unwrap_or_else(|_| "ollama".into()),
            ),
            "copilot" | "github-copilot" => (
                "https://api.githubcopilot.com/chat/completions",
                std::env::var("GITHUB_TOKEN")
                    .or_else(|_| std::env::var("GITHUB_COPILOT_API_KEY"))
                    .unwrap_or_default(),
            ),
            _ => (
                "https://api.openai.com/v1/chat/completions",
                std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            ),
        };

        let resolved_model = match provider.as_str() {
            "ollama" | "copilot" | "github-copilot" => provider_model.to_string(),
            _ => model.to_string(),
        };

        ProviderRoute {
            model: resolved_model,
            base_url: explicit_base_url.unwrap_or(default_base_url).to_string(),
            api_key,
        }
    }
}
