# Search line-range verification record

This is a historical, source-linked record for the search line-range feature. It is not an authorization grant or a substitute for authenticated native task/feature receipts. Later commits must be checked separately; the final feature gate after this record and its verification parent was still pending when this record was written.

## Ubuntu CI

- Run: https://github.com/jared-two-foxes/review/actions/runs/41173381579
- Revision: `01741582c8e62af3ab6e6cce8348da30d2c12f69`
- Event/status/conclusion: `push` / `completed` / `success`
- Runner recorded in logs: Ubuntu 26.04.1.
- Step: **Search range and security regressions**

```text
cargo test -p code-agent-runtime --test search_text --test path_security --test observation_identity
```

The step's logs contain these Unix-specific results:

```text
test search_text_scope_does_not_alias_non_utf8_filenames ... ok
test search_text_working_scope_does_not_follow_symlinks ... ok
test search_text_working_scope_preserves_literal_backslash_filenames ... ok
```

Suite summaries (the full logs include zero ignored and zero filtered-out tests):

| Suite | Passed | Failed |
| --- | ---: | ---: |
| observation_identity | 1 | 0 |
| path_security | 8 | 0 |
| search_text | 14 | 0 |

The existing `cargo build --release` and root-package `cargo test` steps also passed. Plain root-package `cargo test` does not run these runtime integration suites; the explicit added step is necessary. The Unix-only tests above did **not** execute on the local Windows host.

## Local native verification references

The following are local content-addressed receipt identifiers, not public web links. They must be authenticated through the native evidence tools in the originating checkout; this Markdown does not authenticate them.

- Search implementation: `pi-evidence:sha256:dd5035851292a5cf594d1fdbffeb495dfabb446e907fe61dc8097793a37aadce` — `cargo check -p code-agent-runtime` and 24 available Windows search/security/identity regressions passed; independent review approved with a formatting follow-up.
- Formatting follow-up: `pi-evidence:sha256:40418d651b6021f24f0091496600a27d0a15596ecf15c399ed464834bda50c06` — formatting-only change, the same checks passed, independent review approved.
- Original feature gate at `9569e08345e412705bfa72d7df096a92d1963a59`: `pi-feature-evidence:sha256:8661781169bf65252a4fdee4717a63caa7ac7207614ddbd264b88efe54c921e0` — `cargo build --workspace` and `cargo test --workspace` passed; review approved with a Unix-coverage suggestion. This historical result is not final validation of later commits.
- CI-step addition: `pi-evidence:sha256:71ed9e0ce3930bc38a9b4721bb79ecce49c3ef62bca3e259f629729e196b58e3` — 24 available targeted Windows tests passed; independent review approved the two-line workflow addition.
- Verification-parent attempt 1: `pi-evidence:sha256:7891bbd13ea3a0cfbbd2d879ac1bc5a552f18ee17378ef7c67d5536571a192d0` — workspace build, workspace tests, GitHub run metadata retrieval, and GitHub log retrieval all exited zero at revision `01741582c8e62af3ab6e6cce8348da30d2c12f69`. The attempt was **not approved**: its reviewer lacked inspectable execution logs. This record provides compact excerpts for subsequent review; it does not convert that failed review into approval.

Existing compiler warnings and two ignored Copilot-auth tests were retained. No search implementation, test assertions, or workflow behavior is changed by this record.
