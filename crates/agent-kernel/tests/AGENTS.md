# AGENTS.md

## Module summary
`agent-kernel/tests` holds kernel-focused test support.

## Contents
_No child subfolders._

## Key files
- `fake_app.rs` provides a lightweight application double for coordinator and kernel-level tests.

## Find existing code
Start with `fake_app.rs` when you need a minimal `AgentApplication` implementation for kernel tests.

## Add new implementations
Keep test-only fakes and focused kernel test scaffolding here instead of in production modules.
