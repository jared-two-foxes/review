# AGENTS.md

## Module summary
`.github` holds repository automation metadata. In this workspace it is limited to CI definitions that enforce the documented build-and-test contract.

## Contents
| Subfolder | Summary |
| --- | --- |
| `workflows/` | GitHub Actions workflow files for build and test automation. |

## Subsystem interaction
The workflows exercise the Rust workspace from the repository root and act as the shared quality gate for all crates and tests.

## External dependencies used from this level
GitHub Actions runners and marketplace actions such as `actions/checkout` and `dtolnay/rust-toolchain`.
