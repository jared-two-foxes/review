# AGENTS.md

## Module summary
`code-agent-runtime` implements the concrete runtime used by the review application: repository access, diffing, path security, snapshot IDs, target resolution, provider integration, and concrete review tools.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Production runtime modules and tool implementations. |
| `tests/` | Runtime-focused unit and integration tests. |

## Subsystem interaction
`review-app` constructs these tools and adapters, then exposes them to `agent-kernel` through a `ToolCatalog`.

## External dependencies used from this level
`git2` for repository access, `reqwest` for HTTP model calls, `sha2`/`hex` for content IDs, and `glob` for search filtering.
