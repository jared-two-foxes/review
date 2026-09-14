# AGENTS.md

## Module summary
`code-agent-runtime/tests` verifies repository inspection, path security, provider adaptation, and tool behavior.

## Contents
_No child subfolders._

## Key files
- `change_summary.rs`, `changed_files.rs`, and `read_diff.rs` cover diff-oriented tools.
- `read_file.rs`, `list_directory.rs`, `search_text.rs`, and `path_security.rs` cover bounded repository access.
- `provider.rs` and `provider_deadline.rs` cover model-provider adaptation.
- `repo_ops.rs` and `observation_identity.rs` cover repository helpers and snapshot identity.

## Subsystem interaction
These tests validate the runtime surface that `review-app` exposes to the kernel.
