# AGENTS.md

## Module summary
`code-agent-runtime` implements the concrete runtime used by the review application: repository access, diffing, path security, snapshot IDs, target resolution, provider integration, and concrete review tools.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Production runtime modules and tool implementations. |
| `tests/` | Runtime-focused unit and integration tests. |

## Subsystem interaction
`review-app` constructs these tools and adapters, then exposes them to `agent-kernel` through a `ToolCatalog`.

## Find existing code
- Repository and target handling: `/home/runner/work/review/review/crates/code-agent-runtime/src/repo.rs` and `target.rs`
- Tool implementations: `/home/runner/work/review/review/crates/code-agent-runtime/src/tools.rs`
- Security and bounded output: `/home/runner/work/review/review/crates/code-agent-runtime/src/security.rs`
- Model provider adapter: `/home/runner/work/review/review/crates/code-agent-runtime/src/provider.rs`

## Add new implementations
Put concrete repository-backed tool behavior here. If the code is review-policy-specific rather than runtime-generic, keep it in `review-app`.

## Keep in sync
Tool schema or output changes can affect `review-app`, scripted tests, evaluation fixtures, and prompt expectations.

## Search hints
Search for `GitRepo`, `ReviewTarget`, `SecurityPolicy`, `OpenAiProvider`, and tool names beginning with `Get` or `Read`.

## External dependencies used from this level
`git2` for repository access, `reqwest` for HTTP model calls, `sha2`/`hex` for content IDs, and `glob` for search filtering.
