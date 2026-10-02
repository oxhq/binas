# Rust 0.2.0 release candidate

This is an unpublished local candidate. The proposed support profile is bounded
synchronous textual operation on native Windows/MSVC, Node and single-thread
Chromium WASM. Publication and support-profile approval remain separate decisions.
Oxpdf is a minimal consumer, with no editor or UI expansion. The prior frozen
Go CLI/oracle and rollback contract is preserved in
[go-release-surface.md](go-release-surface.md).

## API and behavior relative to cached registry 0.1.1

| Surface | 0.1.1 baseline | 0.2.0 candidate |
| --- | --- | --- |
| Legacy open/inspect/query/byte operations | Available | Retained under scoped regressions |
| Selected document/revision reads | Selected public method absent | Available; warnings mean potentially incomplete |
| Source, snapshot, revision and transaction evidence | Legacy surface | Explicit ownership and versioned coverage |
| Retained native/session JS | Absent | One owner, one pending plan, atomic acceptance, typed lifecycle errors |
| Raw ASCII text | Legacy paths | Conditional retained contract v1, Incremental/Compact |
| Font-aware replacement | Legacy paths | Separate Rust conditional contract; raw JS refuses |
| Limits literals | Older fields | Three new decode cache/live/work fields |
| Exhaustive errors/reports | Older enum/report | Cancelled variant and omitted_document_entries report field |
| C ABI / C FFI | No stable distributed interface | Not implemented; future design only |

The pre-1.0 SemVer choice is **0.2.0**: new public struct fields and exhaustive
enum variants break source compatibility. A 0.1.2 claim would misrepresent this.
See [packaged migration](../crates/binas-pdf/docs/migration.md) and
[session contract](../crates/binas-pdf/docs/pdf-sessions.md). Old JSON Limits/page
reports have serde defaults; that does not preserve Rust literals. Long-running
cloned/repeated reads share cumulative decode work and may now reach limits.
Default page assembly refuses unsupported document entries; explicit policies
report omissions. Legacy verification does not imply broader preservation.

## Contract reconciliation

| Original plan | Executed/proposed candidate | Impact |
| --- | --- | --- |
| P4 worker recovery/cancel and peak memory | Synchronous/cooperative profile; no recovery, mid-call browser cancel or total heap/CPU bound | Support approval needed; broader P4 remains outside candidate |
| P5 diverse independent preservation | Small licensed textual cohort with effects/refusals and external oracles | No universal PDF, forms/tags/accessibility or all-operations claim |
| P6 representative/competitive measurements | Retained/reopen textual equivalents, split phases and bounded independent-session concurrency | No Bento ranking, total copies/allocations/peak memory or broad compression claim |
| Oxpdf public package without overrides | Minimal isolated facade/session consumers from extracted artifacts; explicit sibling patches | Local consumption; public registry resolution follows approved publication |
| Future C ABI/FFI | Direct Rust and generated WASM/JS only | No completed C interface claim |
| P7 product/editor | Excluded | No UI or general PDF expansion |

## Publication proposal and rollback

Publish **binas-core 0.2.0, then binas-pdf 0.2.0, then binas-cli 0.2.0**, waiting
for each dependency's registry availability. `binas-pdf-wasm` stays `publish=false`.
Generated Node/Web bindings are local artifacts, not an npm release. Distribution
of a separate WASM SDK requires its own metadata/notices and explicit approval.

Package allowlists contain production code, README, Apache-2.0 LICENSE and
packaged contracts. Unknown-provenance regression PDFs, checkout-only integration
tests, builds, secrets and evidence are excluded. Private cfg(test) modules still
reference repository fixtures: extracted archives support consumer builds, not
independent execution of the repository test suite.

The packaging harness uses disposable source staging, default Cargo package
verification and patches only to extracted sibling archives. Unpublished version
resolution can modify only the staging lock; the candidate lock remains intact.
Metadata and hashes distinguish this proof from public installation.

After approval, create the matching `rust-v0.2.0` tag and publish under the existing
protected process. Then run minimal Oxpdf with registry resolution and no patches.
Keep 0.1.1 and original inputs/outputs for rollback. This local work performs no
push, tag, PR, GitHub Actions execution or publication.

The current hosted workflow does not provision the producer corpus, independent
Python/Poppler oracles or Node/Chromium runtime qualification. Its optional
external-test entries must not be counted as required corpus passes. This local
candidate proposal requires the recorded P5/oracle/browser/consumer gates for
the exact approved source hash as a pre-publication check. The workflow's package
validation complements that local evidence; hosted multi-OS and public registry
installation remain unproven by this work.
