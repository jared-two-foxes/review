# AGENTS.md

## Module summary
`review-cli/tests` verifies CLI argument handling, schema validity, composition behavior, and live-model integration seams.

## Contents
_No child subfolders._

## Key files
- `v0_cli.rs` and `v0_validation.rs` cover request validation and baseline process-contract behavior.
- `demo_args_composition.rs` and `smoke_composition.rs` cover CLI composition flows.
- `live_model.rs` covers the live-provider integration seam.

## Subsystem interaction
These tests validate the CLI shell without duplicating lower-level runtime and kernel tests.
