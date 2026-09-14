# AGENTS.md

## Module summary
`.github` holds repository automation metadata. In this workspace it is limited to CI definitions that enforce the documented build-and-test contract.

## Contents
| Subfolder | Summary |
| --- | --- |
| `workflows/` | GitHub Actions workflow files for build and test automation. |

## Subsystem interaction
The workflows exercise the Rust workspace from the repository root and act as the shared quality gate for all crates and tests.

## Find existing automation
Start in `/home/runner/work/review/review/.github/workflows` for anything that controls CI behavior or validation expectations.

## Add new automation
Only add repository automation here when it changes contributor workflow, CI, or release behavior; do not place product code in this tree.

## Keep in sync
Workflow command changes should stay aligned with the validation guidance documented in `/home/runner/work/review/review/AGENTS.md` and any related tests or docs.

## Search hints
Look for `cargo build --release`, `cargo test`, `runs-on`, and workflow names.

## External dependencies used from this level
GitHub Actions runners and marketplace actions such as `actions/checkout` and `dtolnay/rust-toolchain`.
