# AGENTS.md

## Module summary
`review-protocol/src/bin` contains small maintenance binaries for protocol artifacts.

## Contents
_No child subfolders._

## Key files
- `generate-schemas.rs` writes the current protocol schemas for check-in under `schemas/review/`.

## Subsystem interaction
This binary depends on the library module in the parent directory and is usually run when protocol structs or validation rules change.

## Usage example
```bash
cargo run -p review-protocol --bin generate-schemas
```
