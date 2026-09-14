# AGENTS.md

## Module summary
`cli-common` centralizes the small pieces of process behavior shared by CLI-facing code: strict JSON reading/writing and stable exit codes.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Shared CLI helper functions and enums. |

## Subsystem interaction
`review-cli` uses this crate to read request files, emit JSON, and return the repository's fixed exit-code contract.

## Find existing code
Go to `/home/runner/work/review/review/crates/cli-common/src/lib.rs` for exit codes and strict JSON helpers.

## Add new implementations
Only place CLI-agnostic process helpers here; argument parsing and review orchestration belong elsewhere.

## Keep in sync
Exit-code changes must stay aligned with ADRs and `review-cli` tests.

## Search hints
Search for `ExitCode`, `read_request`, `write_json_stdout`, and `parse_strict_json`.

## External dependencies used from this level
`serde` and `serde_json` for strict serialization and parsing.
