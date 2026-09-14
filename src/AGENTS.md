# AGENTS.md

## Module summary
`src` holds workspace-level binaries that are not owned by a single crate package.

## Contents
| Subfolder | Summary |
| --- | --- |
| `bin/` | Maintenance binaries executed from the workspace root package. |

## Subsystem interaction
These binaries support repository maintenance tasks that touch shared test artifacts.

## Find existing code
Check `/home/runner/work/review/review/src/bin` for workspace-level maintenance commands.

## Add new implementations
Only place binaries here when they operate across multiple crates or shared fixtures; otherwise prefer crate-local binaries.
