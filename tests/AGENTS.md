# AGENTS.md

## Module summary
`tests` contains workspace-level integration suites that validate protocol drift, deterministic outputs, golden event streams, and evaluation scenarios.

## Contents
| Subfolder | Summary |
| --- | --- |
| `fixtures/` | Checked-in JSON fixtures used by workspace-level tests. |

## Key files
- `schema_drift.rs` ensures generated schemas match the checked-in schema files.
- `v0_determinism.rs` and `v1_golden.rs` verify deterministic result and event outputs.
- `evaluation.rs` seeds fixture repositories and records scripted/live evaluation outcomes.

## Subsystem interaction
These tests sit above individual crates and protect the public contracts and deterministic orchestration behavior of the whole workspace.
