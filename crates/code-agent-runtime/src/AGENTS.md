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

## Subsystem interaction
`tools.rs` sits on top of `repo.rs`, `target.rs`, `diff.rs`, `security.rs`, and `snapshot.rs`; `provider.rs` is independent of repository access and is only consumed when the application needs live model output.

## External dependencies used from this level
Repository access uses `git2`; HTTP provider calls use `reqwest`; bounded content identifiers use `sha2` and `hex`.
