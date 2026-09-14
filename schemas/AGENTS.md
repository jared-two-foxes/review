# AGENTS.md

## Module summary
`schemas` stores checked-in schema artifacts derived from the Rust protocol types.

## Contents
| Subfolder | Summary |
| --- | --- |
| `review/` | Public review request, result, and error schemas. |

## Subsystem interaction
The checked-in files here are verified against `review-protocol` during tests to prevent drift between code and published schema artifacts.

## Find existing artifacts
Look in `/home/runner/work/review/review/schemas/review` for the checked-in public JSON Schemas.

## Add new artifacts
Do not hand-edit generated schemas unless the generation flow explicitly requires it; prefer changing `review-protocol` and regenerating.
