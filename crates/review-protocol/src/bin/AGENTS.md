# AGENTS.md

## Module summary
`review-protocol/src/bin` contains small maintenance binaries for protocol artifacts.

## Contents
_No child subfolders._

## Key files
- `generate-schemas.rs` writes the current protocol schemas for check-in under `schemas/review/`.

## Find existing code
Start with `generate-schemas.rs` when a protocol change requires regenerated checked-in schemas.

## Add new implementations
Keep this directory for maintenance binaries that operate on protocol artifacts rather than runtime review behavior.

## Usage example
```bash
cargo run -p review-protocol --bin generate-schemas
```
