# AGENTS.md

## Module summary
`.github/workflows` defines CI execution for the repository.

## Contents
_No child subfolders._

## Key files
- `ci.yml` builds the release workspace and runs the full test suite on Ubuntu and Windows.

## Find existing automation
Read `/home/runner/work/review/review/.github/workflows/ci.yml` to see the repository's canonical verification commands and runner matrix.

## Add new automation
Keep workflows minimal and repository-wide; crate-specific behavior should usually remain in Rust code or tests rather than in workflow logic.

## Search hints
Search for `matrix`, `cargo build --release`, `cargo test`, `ubuntu-latest`, and `windows-latest`.

## Subsystem interaction
This directory does not call into workspace crates directly; GitHub Actions invokes Cargo commands from the repository root.

## External dependencies used from this level
GitHub-hosted runners plus the checkout and Rust toolchain setup actions.
