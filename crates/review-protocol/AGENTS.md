# AGENTS.md

## Module summary
`review-protocol` defines the public request/result/error data model and the schema-generation helpers that produce the checked-in JSON Schema artifacts.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Protocol types and schema generation code. |

## Subsystem interaction
All request producers and consumers in the workspace rely on this crate's shapes, and `tests/schema_drift.rs` ensures its generated schemas match `schemas/review/`.

## External dependencies used from this level
`serde` and `serde_json` for serialization and schema text generation.
