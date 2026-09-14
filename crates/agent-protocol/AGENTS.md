# AGENTS.md

## Module summary
`agent-protocol` provides small shared protocol primitives for clocks and identifier generation used by deterministic tests and production flows.

## Contents
| Subfolder | Summary |
| --- | --- |
| `src/` | Clock and ID generator implementations. |
| `tests/` | Contract checks for production-oriented protocol sources. |

## Subsystem interaction
`review-app`, tests, and the kernel depend on these abstractions to decouple wall-clock time and ID creation from application logic.

## Find existing code
Look in `/home/runner/work/review/review/crates/agent-protocol/src/lib.rs` for the full clock and ID surface.

## Add new implementations
Only add primitives here when multiple crates need the same deterministic/production time or identity behavior.

## Search hints
Search for `Clock`, `IdGenerator`, `FixedClock`, `SystemClock`, `SequenceIdGenerator`, and `RandomIdGenerator`.

## External dependencies used from this level
`time` for RFC3339 UTC timestamps and `uuid` for production IDs.
