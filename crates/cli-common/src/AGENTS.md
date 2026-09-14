# AGENTS.md

## Module summary
`cli-common/src` contains the entire shared CLI support surface.

## Contents
_No child subfolders._

## Key modules
- `lib.rs` defines exit codes, request file reading, JSON stdout/stderr helpers, and strict JSON parsing.

## Find existing code
Everything in this crate lives in `lib.rs`; read it first before adding overlapping helpers.

## Add new implementations
Keep this directory small and focused on reusable process-level helpers rather than product logic.
