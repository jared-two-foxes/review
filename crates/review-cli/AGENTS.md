# AGENTS.md

## Module summary
`review-cli` is the executable entrypoint for the workspace. It parses CLI flags, validates requests, configures runtime options, and prints the resulting review JSON.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | CLI implementation. |
| `tests/` | CLI integration and validation tests. |

## Subsystem interaction
The binary is a thin shell over `review-app`, `review-protocol`, and `cli-common`, preserving the process boundary described in the ADRs.

## Find existing code
- Argument parsing and exit-code mapping: `/home/runner/work/review/review/crates/review-cli/src/main.rs`
- CLI behavior tests: `/home/runner/work/review/review/crates/review-cli/tests`

## Add new implementations
Put command-line parsing, environment handling, and process behavior here. Shared JSON or exit-code helpers belong in `cli-common`; review logic belongs in `review-app`.

## Keep in sync
CLI flags and emitted behavior must stay aligned with ADRs, `cli-common::ExitCode`, and CLI tests.

## Search hints
Search for `--request`, `--repository`, `--base-ref`, `--head-ref`, `emit_error`, and `ExitCode`.

## External dependencies used from this level
`tracing` and `tracing-subscriber` for logging, plus the workspace crates that perform protocol validation and review execution.

## Usage example
```bash
cargo run -p review-cli -- run --request /abs/path/request.json --format json
```
