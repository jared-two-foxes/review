# AGENTS.md

## Module summary
`review` is a standalone Rust workspace for a review CLI, its protocol contracts, the review application layer, and the supporting agent/runtime primitives it needs to inspect Git repositories and talk to a model provider.

## Repo-wide rules and standards
- Keep the workspace standalone and subprocess-oriented; future platform integration crosses a versioned CLI/JSON boundary instead of importing external production code directly.
- Treat CLI exit codes as a stable public contract and update tests and documentation together with any behavior change.
- Keep checked-in JSON schemas in `schemas/review/` aligned with `review-protocol`; regenerate them with `cargo run -p review-protocol --bin generate-schemas` when protocol shapes change.
- Keep golden fixtures aligned with the current coordinator/review flow; regenerate them with `cargo run --bin regenerate-goldens` when the deterministic event/result shapes change.
- CI validates the workspace with `cargo build --release` and `cargo test` on Ubuntu and Windows, so changes should preserve cross-platform behavior.

## Contents
| Subfolder | Summary |
| --- | --- |
| `.github/` | CI workflow definitions for the workspace. |
| `crates/` | The workspace's library and binary crates. |
| `docs/` | ADRs describing architecture and compatibility boundaries. |
| `schemas/` | Checked-in JSON Schema artifacts for the public review protocol. |
| `scratch/` | Working notes for feature slices and implementation tickets. |
| `src/` | Workspace-level utility binaries, currently golden-fixture regeneration. |
| `tests/` | Cross-crate integration, schema drift, and golden verification tests. |

## Subsystem interaction
- `crates/review-cli` is the entrypoint and translates CLI input into `review-protocol` requests.
- `crates/review-app` composes the review session, wires tools, and drives the `agent-kernel` coordinator.
- `crates/code-agent-runtime` supplies repository, diff, security, and provider adapters used by `review-app`.
- `crates/agent-kernel` and `crates/agent-protocol` provide reusable agent orchestration primitives.
- `tests/`, `schemas/`, and `src/bin/regenerate-goldens.rs` lock down protocol and deterministic behavior.

## Find existing code
- Start in `/home/runner/work/review/review/crates` for all production Rust crates.
- Check `/home/runner/work/review/review/tests` for workspace-level contract tests before changing public behavior.
- Check `/home/runner/work/review/review/docs/decisions` when the change touches architecture boundaries or CLI contracts.

## Add new implementations
- Put reusable runtime or orchestration code in the appropriate crate under `/home/runner/work/review/review/crates`.
- Add workspace-wide maintenance binaries under `/home/runner/work/review/review/src/bin` only when the functionality is not owned by a single crate.
- Keep public protocol changes coordinated across `/home/runner/work/review/review/crates/review-protocol`, `/home/runner/work/review/review/schemas/review`, and `/home/runner/work/review/review/tests`.

## Keep in sync
- `crates/review-protocol` ↔ `schemas/review/` ↔ `tests/schema_drift.rs`
- `src/bin/regenerate-goldens.rs` ↔ `tests/fixtures/` ↔ `tests/v0_determinism.rs` and `tests/v1_golden.rs`
- CLI behavior ↔ ADRs in `docs/decisions/` ↔ `review-cli` tests

## Search hints
- CLI entrypoint: `ReviewConfig`, `run_review`, `main`, `ExitCode`
- Orchestration: `SessionCoordinator`, `AgentApplication`, `ToolCatalog`
- Runtime tools: `GetChangeSummaryTool`, `ReadDiffTool`, `ReadFileTool`, `ListDirectoryTool`, `SearchTextTool`
- Protocol: `ReviewRequest`, `ReviewResult`, `generate_schemas`

## External dependencies used from this level
- Cargo workspace tooling for multi-crate builds and tests.
- `git2` for repository inspection, `reqwest` for model HTTP calls, `serde`/`serde_json` for protocol serialization, `jsonschema` for schema validation, and `tracing` for runtime diagnostics.

## Usage examples
```bash
cargo build --release
cargo test
cargo run -p review-cli -- run --repository /abs/path/to/repo --base-ref HEAD~1 --head-ref HEAD --format json
cargo run -p review-protocol --bin generate-schemas
cargo run --bin regenerate-goldens
```
