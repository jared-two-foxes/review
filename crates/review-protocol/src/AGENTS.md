# AGENTS.md

## Module summary
`review-protocol/src` contains the protocol types and schema emitters.

## Contents
| Subfolder | Summary |
| --- | --- |
| `bin/` | Utility binaries for regenerating checked-in schemas. |

## Key modules
- `lib.rs` defines review status/reason enums, request/result/error structs, request validation, and JSON Schema generation helpers.

## Subsystem interaction
The library is used by both the CLI and application crates, while the schema-generation binary keeps the repository's checked-in artifacts synchronized.
