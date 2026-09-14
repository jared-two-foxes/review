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

## Find existing code
- Review startup and tool wiring: `/home/runner/work/review/review/crates/review-app/src/lib.rs`
- Application-level verification: `/home/runner/work/review/review/crates/review-app/tests`

## Add new implementations
Put review-domain policy, completion validation, and result shaping here. Avoid moving generic kernel behavior or low-level repository utilities into this crate.

## Keep in sync
Changes here often ripple into `review-cli`, `review-protocol`, runtime tools, evaluation tests, and golden fixtures.

## Search hints
Search for `run_review`, `run_review_with_provider`, `ReviewApplication`, `ReviewState`, `ReviewCompletion`, and `Finding`.

## External dependencies used from this level
`agent-kernel`, `agent-protocol`, `code-agent-runtime`, `review-protocol`, `serde`, `serde_json`, and `jsonschema`.
