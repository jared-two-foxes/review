# AGENTS.md

## Module summary
`src/bin` contains workspace maintenance executables.

## Contents
_No child subfolders._

## Key files
- `regenerate-goldens.rs` rebuilds the deterministic golden event/result fixtures under `tests/fixtures/`.

## Subsystem interaction
The binary depends on `review-app`, `agent-kernel`, and `review-protocol` types to keep committed fixtures in sync with coordinator behavior.

## Usage example
```bash
cargo run --bin regenerate-goldens
```
