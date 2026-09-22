use clap::{Arg, ArgAction, Command};
use implement_app::{ImplementConfig, ImplementReason, ImplementRequest, ImplementStatus};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Serialize)]
struct AgentError {
    schema_version: String,
    code: String,
    category: String,
    message: String,
    retryable: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut request_path: Option<String> = None;
    let mut model: String = "opencode/gpt-5.6-terra".into();
    let mut base_url: Option<String> = None;
    let mut max_turns: u32 = 10;
    let mut wall_clock_budget_secs: u64 = 60;
    let mut max_input_tokens: Option<u64> = None;
    let mut max_cost_usd: Option<f64> = None;
    let mut emit_events = false;
    let mut repository: Option<String> = None;
    let mut target_path: Option<String> = None;
    let mut expected_content: Option<String> = None;
    let mut desired_content: Option<String> = None;
    let mut ledger_path: Option<String> = None;

    let mut parsed_run_subcommand = false;
    let mut i = 1;
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
                let value = flag_value(&args, i, "--format");
                if value != "json" {
                    emit_error(
                        "UNSUPPORTED_FORMAT",
                        "validation",
                        &format!("unsupported format: {} (only json is supported)", value),
                        cli_common::ExitCode::InvalidRequest as i32,
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
                max_turns = parse_flag_value(&args, i, "--max-turns", "INVALID_MAX_TURNS");
                i += 1;
            }
            "--wall-clock-budget-secs" => {
                wall_clock_budget_secs = parse_flag_value(
                    &args,
                    i,
                    "--wall-clock-budget-secs",
                    "INVALID_WALL_CLOCK_BUDGET",
                );
                i += 1;
            }
            "--max-input-tokens" => {
                max_input_tokens = Some(parse_flag_value(
                    &args,
                    i,
                    "--max-input-tokens",
                    "INVALID_MAX_INPUT_TOKENS",
                ));
                i += 1;
            }
            "--max-cost-usd" => {
                max_cost_usd = Some(parse_flag_value(
                    &args,
                    i,
                    "--max-cost-usd",
                    "INVALID_MAX_COST_USD",
                ));
                i += 1;
            }
            "--emit-events" => {
                emit_events = true;
            }
            "--repository" => {
                repository = Some(flag_value(&args, i, "--repository"));
                i += 1;
            }
            "--target-path" => {
                target_path = Some(flag_value(&args, i, "--target-path"));
                i += 1;
            }
            "--expected-content" => {
                expected_content = Some(flag_value(&args, i, "--expected-content"));
                i += 1;
            }
            "--desired-content" => {
                desired_content = Some(flag_value(&args, i, "--desired-content"));
                i += 1;
            }
            "--ledger" => {
                ledger_path = Some(flag_value(&args, i, "--ledger"));
                i += 1;
            }
            _ => emit_error(
                "UNRECOGNIZED_ARGUMENT",
                "validation",
                &format!("unrecognized argument: {}", args[i]),
                cli_common::ExitCode::InvalidRequest as i32,
            ),
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

    let request: ImplementRequest = if let Some(path) = request_path {
        let bytes = match cli_common::read_request(Path::new(&path)) {
            Ok(bytes) => bytes,
            Err(error) => emit_error(
                "REQUEST_READ_FAILED",
                "validation",
                &error.to_string(),
                cli_common::ExitCode::InvalidRequest as i32,
            ),
        };
        match cli_common::parse_strict_json(&bytes) {
            Ok(request) => request,
            Err(cli_common::ParseError::Syntax(_)) => {
                emit_error(
                    "REQUEST_PARSE_FAILED",
                    "validation",
                    "request is not valid JSON",
                    cli_common::ExitCode::InvalidRequest as i32,
                );
            }
            Err(cli_common::ParseError::DuplicateKey(key)) => {
                emit_error(
                    "DUPLICATE_KEY",
                    "validation",
                    &format!("duplicate key: {}", key),
                    cli_common::ExitCode::InvalidRequest as i32,
                );
            }
        }
    } else {
        ImplementRequest {
            repository_path: required_value(
                repository,
                "MISSING_REPOSITORY",
                "--repository or --request is required",
            ),
            target_path: required_value(
                target_path,
                "MISSING_TARGET_PATH",
                "--target-path or --request is required",
            ),
            expected_content: required_value(
                expected_content,
                "MISSING_EXPECTED_CONTENT",
                "--expected-content or --request is required",
            ),
            desired_content: required_value(
                desired_content,
                "MISSING_DESIRED_CONTENT",
                "--desired-content or --request is required",
            ),
        }
    };

    let config = ImplementConfig {
        api_key: None,
        model,
        base_url,
        max_turns,
        max_tool_calls: 10,
        max_completion_attempts: 3,
        wall_clock_budget: Some(Duration::from_secs(wall_clock_budget_secs)),
        ledger_path: ledger_path.map(PathBuf::from),
        max_repeated_actions: 3,
        max_input_tokens,
        max_cost_usd,
    };

    if let Err(message) = implement_app::validate_request(&request) {
        emit_error(
            "INVALID_REQUEST",
            "validation",
            &message,
            cli_common::ExitCode::InvalidRequest as i32,
        );
    }

    let cancellation_token = Arc::new(AtomicBool::new(false));
    let token_clone = Arc::clone(&cancellation_token);
    if let Err(error) = ctrlc::set_handler(move || {
        token_clone.store(true, Ordering::Relaxed);
    }) {
        emit_error(
            "SIGNAL_HANDLER_INSTALL_FAILED",
            "runtime",
            &error.to_string(),
            cli_common::ExitCode::InternalFailure as i32,
        );
    }

    let (result, _events) =
        match implement_app::run_implement(&request, &config, Some(&cancellation_token)) {
            Ok(value) => value,
            Err(message) => match message.as_str() {
                "missing API key" => emit_error(
                    "MISSING_API_KEY",
                    "configuration",
                    &message,
                    cli_common::ExitCode::InternalFailure as i32,
                ),
                _ if message.contains("unsupported model provider prefix") => emit_error(
                    "INVALID_ARGUMENTS",
                    "validation",
                    &message,
                    cli_common::ExitCode::InvalidRequest as i32,
                ),
                _ => emit_error(
                    "IMPLEMENT_SETUP_FAILED",
                    "runtime",
                    &message,
                    cli_common::ExitCode::InternalFailure as i32,
                ),
            },
        };

    cli_common::write_json_stdout(&result).expect("failed to write result");
    std::process::exit(exit_code_for_result(&result));
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
        HelpTarget::Root => print_help(implement_cli_command()),
        HelpTarget::Run => {
            let mut command = implement_cli_command();
            let run = command
                .find_subcommand_mut("run")
                .expect("implement-cli help must define the run subcommand")
                .clone();
            print_help(run);
        }
    }
}

fn implement_cli_command() -> Command {
    Command::new("implement-cli")
        .about("Run implementation requests against a repository")
        .subcommand(
            Command::new("run")
                .about("Execute an implementation request")
                .arg(
                    Arg::new("request")
                        .long("request")
                        .value_name("PATH")
                        .help("Read an implementation request JSON file"),
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
                    Arg::new("target-path")
                        .long("target-path")
                        .value_name("PATH")
                        .help("Target file path to update"),
                )
                .arg(
                    Arg::new("expected-content")
                        .long("expected-content")
                        .value_name("TEXT")
                        .help("Expected current file contents"),
                )
                .arg(
                    Arg::new("desired-content")
                        .long("desired-content")
                        .value_name("TEXT")
                        .help("Desired replacement file contents"),
                )
                .arg(
                    Arg::new("ledger")
                        .long("ledger")
                        .value_name("PATH")
                        .help("Write a run ledger to this path"),
                ),
        )
}

fn emit_error(code: &str, category: &str, message: &str, exit_code: i32) -> ! {
    let error = AgentError {
        schema_version: "agent.error/v1".into(),
        code: code.into(),
        category: category.into(),
        message: message.into(),
        retryable: false,
    };
    let _ = cli_common::write_json_stdout(&error);
    std::process::exit(exit_code);
}

fn required_value(value: Option<String>, code: &str, message: &str) -> String {
    match value {
        Some(value) => value,
        None => emit_error(
            code,
            "validation",
            message,
            cli_common::ExitCode::InvalidRequest as i32,
        ),
    }
}

fn flag_value(args: &[String], index: usize, flag: &str) -> String {
    match args.get(index + 1) {
        Some(value) => value.clone(),
        None => emit_error(
            "MISSING_ARGUMENT_VALUE",
            "validation",
            &format!("{} requires a value", flag),
            cli_common::ExitCode::InvalidRequest as i32,
        ),
    }
}

fn parse_flag_value<T: std::str::FromStr>(
    args: &[String],
    index: usize,
    flag: &str,
    error_code: &str,
) -> T {
    let raw = flag_value(args, index, flag);
    raw.parse().unwrap_or_else(|_| {
        emit_error(
            error_code,
            "validation",
            &format!("{} requires a valid value", flag),
            cli_common::ExitCode::InvalidRequest as i32,
        )
    })
}

fn exit_code_for_result(result: &implement_app::ImplementResult) -> i32 {
    match &result.reason {
        ImplementReason::Cancelled => cli_common::ExitCode::Cancellation as i32,
        _ => match &result.status {
            ImplementStatus::CandidateReady => 0,
            ImplementStatus::Indeterminate => cli_common::ExitCode::Indeterminate as i32,
        },
    }
}
