# AGENTS.md

## Module summary
`.github/workflows` defines CI execution for the repository.

## Contents
_No child subfolders._

## Key files
- `ci.yml` builds the release workspace and runs the full test suite on Ubuntu and Windows.

## Subsystem interaction
This directory does not call into workspace crates directly; GitHub Actions invokes Cargo commands from the repository root.

## External dependencies used from this level
GitHub-hosted runners plus the checkout and Rust toolchain setup actions.

## Usage example
```yaml
# Triggered automatically on push and pull_request.
```
