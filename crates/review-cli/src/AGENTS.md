# AGENTS.md

## Module summary
`review-cli/src` contains the CLI program logic.

## Contents
_No child subfolders._

## Key modules
- `main.rs` parses flags, loads or constructs `ReviewRequest`, validates it, runs `review-app`, emits JSON, and maps review status to exit codes.

## Find existing code
Everything in this crate's production surface is in `main.rs`; start there for flags, env vars, request construction, and exit handling.

## Add new implementations
Keep user-facing CLI behavior here. If a change could be reused by other binaries, move the shared part into `cli-common` or a lower-level crate.
