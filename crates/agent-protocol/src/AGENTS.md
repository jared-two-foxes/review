# AGENTS.md

## Module summary
`agent-protocol/src` implements injectable time and ID sources.

## Contents
_No child subfolders._

## Key modules
- `lib.rs` defines the `Clock` and `IdGenerator` traits plus fixed/system clock and sequential/random ID implementations.

## Subsystem interaction
Deterministic tests use the fixed and sequence implementations; production paths use the system clock and UUID generator.
