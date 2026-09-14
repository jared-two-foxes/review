# AGENTS.md

## Module summary
`tests/fixtures/v1` contains golden event and result fixtures for scripted coordinator scenarios.

## Contents
_No child subfolders._

## Key files
- `golden-events-scenario-*.json` capture coordinator ledger output.
- `golden-result-scenario-*.json` capture final review results for the same scenarios.

## Find existing fixtures
Use scenario-numbered files here when adjusting deterministic multi-turn coordinator behavior.

## Add new artifacts
Regenerate these fixtures together with the corresponding changes in the coordinator or review application.

## Keep in sync
`tests/v1_golden.rs` and `src/bin/regenerate-goldens.rs` use these artifacts together.
