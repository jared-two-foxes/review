# AGENTS.md

## Module summary
`review-protocol` defines the public request/result/error data model and the schema-generation helpers that produce the checked-in JSON Schema artifacts.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Protocol types and schema generation code. |

## Subsystem interaction
All request producers and consumers in the workspace rely on this crate's shapes, and `tests/schema_drift.rs` ensures its generated schemas match `schemas/review/`.

## Find existing code
Read `/home/runner/work/review/review/crates/review-protocol/src/lib.rs` for request/result/error shapes and `/home/runner/work/review/review/crates/review-protocol/src/bin/generate-schemas.rs` for schema regeneration.

## Add new implementations
Put public contract types and schema emitters here; keep repository-specific runtime logic out of this crate.

## Keep in sync
Any change here usually requires matching updates in `schemas/review/`, CLI/application consumers, and schema drift tests.

## Search hints
Search for `ReviewRequest`, `ReviewResult`, `AgentError`, `validate_request`, and `generate_schemas`.

## External dependencies used from this level
`serde` and `serde_json` for serialization and schema text generation.
