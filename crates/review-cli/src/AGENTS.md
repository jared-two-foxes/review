# AGENTS.md

## Module summary
`review-cli/src` contains the CLI program logic.

## Contents
_No child subfolders._

## Key modules
- `main.rs` parses flags, loads or constructs `ReviewRequest`, validates it, runs `review-app`, emits JSON, and maps review status to exit codes.

## Subsystem interaction
This source directory depends on `cli-common` for JSON/process helpers and delegates actual review execution to `review-app`.
