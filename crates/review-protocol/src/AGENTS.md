# AGENTS.md

## Module summary
`review-protocol/src` contains the protocol types and schema emitters.

## Contents
| Subfolder | Summary |
| --- | --- |
| `bin/` | Utility binaries for regenerating checked-in schemas. |

## Key modules
- `lib.rs` defines review status/reason enums, request/result/error structs, request validation, and JSON Schema generation helpers.

## Find existing code
- Public types and validation: `lib.rs`
- Schema regeneration command: `bin/generate-schemas.rs`

## Add new implementations
Add public protocol structs and validation rules here; keep operational review flow elsewhere.

## Keep in sync
Protocol changes must stay aligned with checked-in schemas and any consumer tests.

## Search hints
Search for `validate_request`, `request_schema`, `review_schema`, and `error_schema`.

## Subsystem interaction
The library is used by both the CLI and application crates, while the schema-generation binary keeps the repository's checked-in artifacts synchronized.
