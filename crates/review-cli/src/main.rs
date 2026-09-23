use clap::{Arg, ArgAction, Command};
use code_agent_runtime::provider::resolve_provider_route_with_root;
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
    let mut model: String = "opencode/gpt-5.6-terra".into();
    let mut provider_root: Option<String> = None;
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

    let mut parsed_run_subcommand = false;
    let mut i = 1; // Skip program name
    while i < args.len() {
        if let Some(target) = requested_help(&args, i, parsed_run_subcommand) {
            print_requested_help(target);
        }

        match args[i].as_str() {
            "run" => {
                parsed_run_subcommand = true;
            }
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
                provider_root = Some(flag_value(&args, i, "--base-url"));
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

    if let Err(error) = resolve_provider_route_with_root(&model, None, provider_root.as_deref()) {
        emit_error("INVALID_ARGUMENTS", &error);
    }

    let config = ReviewConfig {
        model,
        provider_root,
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

enum HelpTarget {
    Root,
    Run,
}

fn requested_help(
    args: &[String],
    index: usize,
    parsed_run_subcommand: bool,
) -> Option<HelpTarget> {
    match args.get(index).map(String::as_str) {
        Some("--help") | Some("-h") => Some(if parsed_run_subcommand {
            HelpTarget::Run
        } else {
            HelpTarget::Root
        }),
        Some("help") => Some(
            if parsed_run_subcommand || args.get(index + 1).is_some_and(|arg| arg == "run") {
                HelpTarget::Run
            } else {
                HelpTarget::Root
            },
        ),
        _ => None,
    }
}

fn print_help(mut command: Command) -> ! {
    command
        .print_long_help()
        .expect("failed to write help output");
    println!();
    std::process::exit(0);
}

fn print_requested_help(target: HelpTarget) -> ! {
    match target {
        HelpTarget::Root => print_help(review_cli_command()),
        HelpTarget::Run => {
            let mut command = review_cli_command();
            let run = command
                .find_subcommand_mut("run")
                .expect("review-cli help must define the run subcommand")
                .clone();
            print_help(run);
        }
    }
}

fn review_cli_command() -> Command {
    Command::new("review-cli")
        .about("Run review requests against a repository diff")
        .subcommand(
            Command::new("run")
                .about("Execute a review request")
                .arg(
                    Arg::new("request")
                        .long("request")
                        .value_name("PATH")
                        .help("Read a review request JSON file"),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_name("FORMAT")
                        .help("Output format (only json is supported)"),
                )
                .arg(
                    Arg::new("model")
                        .long("model")
                        .value_name("MODEL")
                        .help("Model name or provider-prefixed model route"),
                )
                .arg(
                    Arg::new("base-url")
                        .long("base-url")
                        .value_name("URL")
                        .help("Override the provider base URL"),
                )
                .arg(
                    Arg::new("max-turns")
                        .long("max-turns")
                        .value_name("COUNT")
                        .help("Maximum model turns before stopping"),
                )
                .arg(
                    Arg::new("wall-clock-budget-secs")
                        .long("wall-clock-budget-secs")
                        .value_name("SECONDS")
                        .help("Wall-clock budget in seconds"),
                )
                .arg(
                    Arg::new("max-input-tokens")
                        .long("max-input-tokens")
                        .value_name("TOKENS")
                        .help("Abort after exceeding this many input tokens"),
                )
                .arg(
                    Arg::new("max-cost-usd")
                        .long("max-cost-usd")
                        .value_name("USD")
                        .help("Abort after exceeding this estimated USD cost"),
                )
                .arg(
                    Arg::new("emit-events")
                        .long("emit-events")
                        .action(ArgAction::SetTrue)
                        .help("Enable info-level event logging on stderr"),
                )
                .arg(
                    Arg::new("repository")
                        .long("repository")
                        .value_name("PATH")
                        .help("Repository path used when constructing a request from flags"),
                )
                .arg(
                    Arg::new("base-ref")
                        .long("base-ref")
                        .value_name("REF")
                        .help("Base ref for the review request"),
                )
                .arg(
                    Arg::new("head-ref")
                        .long("head-ref")
                        .value_name("REF")
                        .help("Head ref for the review request"),
                )
                .arg(
                    Arg::new("requirements")
                        .long("requirements")
                        .value_name("PATH")
                        .help("Path to a requirements file for demo-style requests"),
                )
                .arg(
                    Arg::new("uncommitted")
                        .long("uncommitted")
                        .action(ArgAction::SetTrue)
                        .help("Use the working tree as the head ref"),
                )
                .arg(
                    Arg::new("ledger")
                        .long("ledger")
                        .value_name("PATH")
                        .help("Write a run ledger to this path"),
                ),
        )
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
