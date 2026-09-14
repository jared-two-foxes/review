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

## Find existing tests
- Schema contract checks: `schema_drift.rs`
- Deterministic output checks: `v0_determinism.rs` and `v1_golden.rs`
- Scenario evaluation harness: `evaluation.rs`
- Fixture data: `/home/runner/work/review/review/tests/fixtures`

## Add new implementations
Add workspace-level tests here when the behavior spans multiple crates or validates repository-wide contracts.

## Keep in sync
If protocol, CLI, or deterministic coordinator behavior changes, update both these tests and the fixtures or schemas they assert against.

## Search hints
Search for `golden`, `schema_drift`, `evaluation-output`, and fixture scenario names.
