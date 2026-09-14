# AGENTS.md

## Module summary
`code-agent-runtime/src` contains the concrete implementation of repository-backed review tooling.

## Contents
_No child subfolders._

## Key modules
- `diff.rs` computes changed files and bounded diff output.
- `error.rs` defines repository/runtime error types.
- `provider.rs` adapts the model API into the kernel's canonical provider interface.
- `repo.rs` opens repositories and canonicalizes paths safely.
- `security.rs` enforces path and output bounds for file, directory, and search access.
- `snapshot.rs` derives stable snapshot identifiers for base/head pairs.
- `target.rs` resolves review targets such as commits and working-tree views.
- `tools.rs` implements change summary, changed-file, diff, file, directory, and text-search tools.
- `lib.rs` re-exports the runtime modules.

## Find existing code
- Diff computation: `diff.rs`
- Safe repository access: `repo.rs` and `security.rs`
- Review target parsing and snapshot identity: `target.rs` and `snapshot.rs`
- Concrete tool entrypoints: `tools.rs`
- Remote model wiring: `provider.rs`

## Add new implementations
- New repository-reading tools should usually start in `tools.rs` and reuse helpers from `repo.rs`, `diff.rs`, `target.rs`, `security.rs`, and `snapshot.rs`.
- Provider-specific HTTP integration belongs in `provider.rs`.
- Path validation or truncation rules belong in `security.rs`.

## Keep in sync
If a tool's request/response shape changes, update the review application prompts, runtime tests, and any deterministic fixtures that depend on it.

## Search hints
Search for `compute_diff`, `changed_files`, `read_diff`, `canonicalize_path`, `validate_path`, `snapshot_id`, and `normalize_usage`.

## Subsystem interaction
`tools.rs` sits on top of `repo.rs`, `target.rs`, `diff.rs`, `security.rs`, and `snapshot.rs`; `provider.rs` is independent of repository access and is only consumed when the application needs live model output.

## External dependencies used from this level
Repository access uses `git2`; HTTP provider calls use `reqwest`; bounded content identifiers use `sha2` and `hex`.
