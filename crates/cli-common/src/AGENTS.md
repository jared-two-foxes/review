# AGENTS.md

## Module summary
`cli-common/src` contains the entire shared CLI support surface.

## Contents
_No child subfolders._

## Key modules
- `lib.rs` defines exit codes, request file reading, JSON stdout/stderr helpers, and strict JSON parsing.

## Subsystem interaction
The helpers keep CLI process behavior small and predictable so higher layers can focus on review logic.
