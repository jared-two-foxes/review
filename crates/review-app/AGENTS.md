# AGENTS.md

## Module summary
`review-app` is the review-specific application layer. It turns protocol requests into a configured kernel session, validates completions, and maps final state into review results.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Production review composition and review-specific application logic. |
| `tests/` | End-to-end application tests. |

## Subsystem interaction
This crate is the central integration point: it depends on protocol crates for request/result shapes, the kernel for orchestration, and the runtime crate for repository/provider tools.

## External dependencies used from this level
`agent-kernel`, `agent-protocol`, `code-agent-runtime`, `review-protocol`, `serde`, `serde_json`, and `jsonschema`.
