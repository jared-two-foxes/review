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

## External dependencies used from this level
`tracing` and `tracing-subscriber` for logging, plus the workspace crates that perform protocol validation and review execution.

## Usage example
```bash
cargo run -p review-cli -- run --request /abs/path/request.json --format json
```
