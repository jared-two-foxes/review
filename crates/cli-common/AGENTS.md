# AGENTS.md

## Module summary
`cli-common` centralizes the small pieces of process behavior shared by CLI-facing code: strict JSON reading/writing and stable exit codes.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Shared CLI helper functions and enums. |

## Subsystem interaction
`review-cli` uses this crate to read request files, emit JSON, and return the repository's fixed exit-code contract.

## External dependencies used from this level
`serde` and `serde_json` for strict serialization and parsing.
