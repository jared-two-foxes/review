# AGENTS.md

## Module summary
`tests/fixtures/v0` contains minimal fixtures for the baseline deterministic V0 flow.

## Contents
_No child subfolders._

## Key files
- `minimal-request.json` is the canonical minimal request fixture.
- `golden-result.json` is the expected deterministic result payload for that request.

## Subsystem interaction
`v0_determinism.rs` and the regeneration binary use these files to verify that the baseline protocol output stays stable.
