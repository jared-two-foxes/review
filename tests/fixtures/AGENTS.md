# AGENTS.md

## Module summary
`tests/fixtures` stores checked-in artifacts consumed by workspace-level tests.

## Contents
| Subfolder | Summary |
| --- | --- |
| `v0/` | Baseline V0 request/result fixtures. |
| `v1/` | Golden event and result fixtures for multi-turn coordinator scenarios. |

## Subsystem interaction
The regeneration binary and golden/schema tests read and rewrite files here when deterministic outputs intentionally change.

## Find existing fixtures
Use `v0/` for baseline request/result artifacts and `v1/` for multi-turn coordinator goldens.

## Add new artifacts
Keep fixture additions aligned with the specific test that consumes them and document regeneration commands when the files are derived.
