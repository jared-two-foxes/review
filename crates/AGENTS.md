# AGENTS.md

## Module summary
`crates` contains the runtime, protocol, orchestration, application, CLI, and support crates that make up the review workspace.

## Contents
| Subfolder | Summary |
| --- | --- |
| `agent-kernel/` | Generic agent application, coordinator, ledger, model, and tool orchestration primitives. |
| `agent-protocol/` | Shared clock and ID-generation abstractions for deterministic and production flows. |
| `cli-common/` | Small helpers for strict JSON I/O and process exit codes. |
| `code-agent-runtime/` | Repository, diff, security, snapshot, provider, and review-tool implementations. |
| `review-app/` | Review-specific application composition and completion/result shaping. |
| `review-cli/` | End-user CLI entrypoint for running reviews. |
| `review-protocol/` | Public request/result/error schemas and protocol validation helpers. |

## Subsystem interaction
`review-cli` depends on `review-app` and `review-protocol`; `review-app` depends on `agent-kernel`, `agent-protocol`, `code-agent-runtime`, and `review-protocol`; `code-agent-runtime` depends on `agent-kernel`; the remaining crates provide protocol or utility support around that spine.

## Find existing code
- CLI behavior: `/home/runner/work/review/review/crates/review-cli`
- Review composition and completion rules: `/home/runner/work/review/review/crates/review-app`
- Repository tools and model adapters: `/home/runner/work/review/review/crates/code-agent-runtime`
- Shared orchestration primitives: `/home/runner/work/review/review/crates/agent-kernel`
- Request/result contracts: `/home/runner/work/review/review/crates/review-protocol`

## Add new implementations
- Put domain-agnostic agent plumbing in `agent-kernel`.
- Put review-specific flow logic in `review-app`.
- Put concrete repository access and tool execution code in `code-agent-runtime`.
- Put CLI-only parsing and process concerns in `review-cli` or `cli-common`.

## Keep in sync
Protocol or contract changes often require updates in `review-protocol`, `review-cli`, `review-app`, `schemas/review/`, and workspace tests.

## Search hints
Search crate names first, then symbols such as `SessionCoordinator`, `ReviewApplication`, `GitRepo`, `OpenAiProvider`, `ExitCode`, and `generate_schemas`.

## External dependencies used from this level
This layer concentrates most third-party Rust dependencies, including `serde`, `jsonschema`, `git2`, `reqwest`, `time`, `uuid`, and `tracing`.

## Usage example
```bash
cargo test -p review-cli
cargo test -p review-app
```
