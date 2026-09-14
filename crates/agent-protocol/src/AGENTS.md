# AGENTS.md

## Module summary
`agent-protocol/src` implements injectable time and ID sources.

## Contents
_No child subfolders._

## Key modules
- `lib.rs` defines the `Clock` and `IdGenerator` traits plus fixed/system clock and sequential/random ID implementations.

## Find existing code
All production and deterministic protocol primitives in this crate live in `lib.rs`.

## Add new implementations
Add new sources only if callers need them across crate boundaries; otherwise keep test-specific fakes near their tests.
