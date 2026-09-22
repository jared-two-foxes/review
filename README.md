# review

`review` is a standalone Rust workspace for an AI-assisted code review tool and the reusable runtime underneath it.

The project is centered on a `review` CLI that:

- accepts a versioned review request
- compares two repository states
- exposes bounded, read-only repository tools to a model
- returns a structured review result

This repository is also an experiment in building the shared pieces needed for agentic developer tools: a generic agent kernel, a code-oriented runtime, and product-specific review logic.

## What the project does

At a high level, the system is designed to review code changes by combining:

- a **CLI process contract** for callers and automation
- a **review application** that defines review-specific policy
- a **bounded agent loop** that coordinates model turns and tool calls
- **read-only repository tools** for inspecting diffs, files, directories, and text matches
- **typed JSON protocols** for requests, results, errors, and schemas

Today, the codebase already includes a runnable CLI, protocol schemas, deterministic test seams, repository tooling, and an OpenAI-compatible provider adapter. The repository also documents an intentionally narrow V0 boundary and a broader implementation roadmap.

## Architecture

The main crates are:

- `crates/review-cli` — CLI entrypoint and argument parsing
- `crates/review-app` — review-specific behavior, completion rules, and built-in review skills
- `crates/implement-app` — proof-of-concept second consumer for scoped-write implementation flows
- `crates/implement-cli` — CLI entrypoint for implement-app
- `crates/agent-kernel` — generic bounded model/tool orchestration
- `crates/code-agent-runtime` — repository access, diffs, security boundaries, and model provider integration
- `crates/review-protocol` — request/result/error types and JSON Schema generation
- `crates/agent-protocol` — clock and identifier abstractions
- `crates/cli-common` — shared CLI JSON I/O and exit codes

Supporting directories include:

- `schemas/review` — checked-in JSON Schemas
- `tests` — workspace-level golden, schema, and evaluation tests
- `docs/decisions` — architecture decision records
- `src/bin/regenerate-goldens.rs` — helper for regenerating golden fixtures

## Key review capabilities

The review flow is built around bounded, read-only inspection tools such as:

- change summary
- changed file enumeration
- per-file diff reading
- file reading
- directory listing
- text search

The runtime includes explicit safety constraints, including denied sensitive paths, bounded output sizes, and completeness signals for truncated results.

## CLI shape

The `review-cli` binary supports either:

- a JSON request file via `--request`
- or demo-style request construction via flags like `--repository`, `--base-ref`, and `--head-ref`

Common flags include:

- `--request`
- `--repository`
- `--base-ref`
- `--head-ref`
- `--requirements`
- `--model`
- `--base-url`
- `--max-turns`
- `--wall-clock-budget-secs`

Provider routing is model-driven:

- `--model opencode/gpt-5.6-terra` (or any unprefixed model) routes to OpenAI (`OPENCODE_API_KEY`)
- `--model openai/<model-name>` routes to OpenAI (`OPENAI_API_KEY`)
- `--model ollama/<model-name>` routes to Ollama (`http://127.0.0.1:11434/v1/chat/completions`)
- `--model opencode/<model-name>` routes to OpenCode (`https://opencode.ai/zen/v1/chat/completions`, `OPENCODE_API_KEY`)
- `--model copilot/<model-name>` or `--model github-copilot/<model-name>` routes to GitHub Copilot (`GITHUB_TOKEN`)

The `implement-cli` binary follows the same provider-routing model and runtime flags, but builds `ImplementRequest` values instead. It supports either:

- a JSON request file via `--request`
- or demo-style request construction via flags like `--repository`, `--target-path`, `--expected-content`, and `--desired-content`

## Example usage

Install the two CLIs directly with `cargo install`:

```bash
cargo install --git https://github.com/jared-two-foxes/review --locked --package review-cli
cargo install --git https://github.com/jared-two-foxes/review --locked --package implement-cli
```

Run the CLI against a request fixture:

```bash
cargo run -p review-cli -- run --request tests/fixtures/v0/minimal-request.json --format json
```

Run the CLI by constructing the request from flags:

```bash
cargo run -p review-cli -- run \
  --repository . \
  --base-ref HEAD~1 \
  --head-ref HEAD \
  --format json
```

Run the implement CLI by constructing an implementation request from flags:

```bash
cargo run -p implement-cli -- run \
  --repository . \
  --target-path README.md \
  --expected-content "before" \
  --desired-content "after" \
  --format json
```

To use a live provider path, set `OPENAI_API_KEY`. Optional environment variables used in tests and live smoke flows include:

- `OPENAI_API_KEY`
- `REVIEW_MODEL`
- `REVIEW_BASE_URL`
- `REVIEW_LIVE_TEST`

## Development and testing

Build the workspace:

```bash
cargo build --release
```

Run the test suite:

```bash
cargo test
```

CI currently runs both commands on Ubuntu and Windows.

## Repository status

This repository is intentionally structured as a greenfield standalone workspace. The decision records describe two important boundaries:

- the CLI contract and exit-code behavior are treated as stable
- any future Scaffold integration is intended to remain subprocess-only rather than a direct code dependency

The broader implementation plan in `agent-platform-vertical-implementation-plan.md` shows that Review is both a product experiment and the first consumer of the shared agent platform components.

## Where to start

If you are exploring the repository for the first time, a good reading order is:

1. `docs/decisions/0001-v0-process-contract.md`
2. `agent-platform-vertical-implementation-plan.md`
3. `crates/review-cli/src/main.rs`
4. `crates/review-app/src/lib.rs`
5. `crates/agent-kernel/src/coordinator.rs`
6. `crates/code-agent-runtime/src/tools.rs`
