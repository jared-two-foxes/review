use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut request_path: Option<&str> = None;
    let mut i = 1; // Skip program name
    while i < args.len() {
        match args[i].as_str() {
            "run" => { /* subcommand marker, no-op for V0 */ }
            "--request" => {
                request_path = Some(&args[i + 1]);
                i += 1;
            }
            "--format" => {
                // V0 only supports JSON; accept and ignore for now.
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let path = request_path.expect("--request is required");

    let bytes = match cli_common::read_request(Path::new(path)) {
        Ok(b) => b,
        Err(e) => return emit_error("REQUEST_READ_FAILED", &e.to_string()),
    };

    let request: review_protocol::ReviewRequest = match cli_common::parse_strict_json(&bytes) {
        Ok(r) => r,
        Err(cli_common::ParseError::Syntax(_e)) => {
            return emit_error("REQUEST_PARSE_FAILED", "request is not valid JSON");
        }
        Err(cli_common::ParseError::DuplicateKey(key)) => {
            return emit_error("DUPLICATE_KEY", &format!("duplicate key: {}", key));
        }
    };

    if let Err(msg) = review_protocol::validate_request(&request) {
        return emit_error("REQUEST_VALIDATION_FAILED", &msg);
    }

    let result = review_app::run_review(&request);

    cli_common::write_json_stdout(&result).expect("failed to write result");

    std::process::exit(cli_common::ExitCode::Indeterminate as i32);
}

fn emit_error(code: &str, message: &str) {
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
