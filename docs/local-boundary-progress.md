# Local h4x boundary progress: P3 text cohort

This is local development evidence for the h4x boundary plan, not the support
contract of the published `binas-pdf 0.1.1`. Package versions have not changed.
Release version and support scope must be reviewed before publication.

| Package | Local outcome | Remaining boundary |
| --- | --- | --- |
| P0 | Frozen baseline and reproducible source provenance | Release packaging still pending |
| P1 / P1.1 | Selective extraction and page geometry | No universal layout or text semantics claim |
| P2a | Public operation budget, cancellation, source/snapshot and revision controls | Session lifetime surface is P4 |
| P2b | Narrow raw ASCII text capability negotiation and runtime result acceptance, Incremental/Compact | Font-aware page edits use the existing page-text API; no new negotiated capability |
| P3a | Independent structural preservation for the admitted plain/Flate raw text cohort; unchanged valid pages render identically | Fontless target appearance explicitly unverified |
| P3b | Independent font/CMap, exact text and visible golden-edit evidence for one admitted fonted plain/Flate cohort; explicit refusals | Cohort closed locally; no general font, producer, reflow or accessibility guarantee |
| P4 | Pending | Bounded native/WASM retained sessions, limits, cancellation, disposal and snapshot parity |
| P5 | Pending | Additional compatibility/producer cases and oracles with licensed provenance |
| P6 | Pending | Performance, artifact/package contents, version/support review and migration; public crate acceptance in minimal Oxpdf consumer after authorized release |
| P7 | Outside this scope | Editor/UI expansion |

P3 is closed for these two finite text cohorts, not for every operation already
present in Binas. Other operation-specific preservation profiles are unclaimed.
Additional fonts/producers are P5 qualification work rather than an endless P3
gate. No production change was needed in P3a/P3b: the existing operation passes.

P3b replaces the first `ALPHA` with `OMEGA-LONG` on page zero of an original
synthetic three-page PDF. The target has a unique content stream and a shared
Helvetica/WinAnsi font with an explicit injective ToUnicode CMap. Only target
stream payload and Length change. All other typed objects, encoded/decoded
non-target streams, pages, geometry, metadata, annotations, attachment, outline,
name tree, shared resource/font/CMap and unreachable sentinel remain exact.
Missing CMap, unmapped replacement, ambiguous reverse mapping and shared target
content with Reject fail before a revision is published.

The independent pypdf strict reader and Poppler pdfinfo/renderer are installed
local tools, invoked by the QA commands in `qa/fonted-text/README.md`. The target
render equals an independently generated expected edited PDF; source-to-golden
change is nonempty and confined to a declared region. Unchanged pages retain
source pixels. Both modes are checked for plain and Flate sources. Named negative
mutations exercise wrong font, lost/collateral text, shared resources/CMap,
image/sentinel damage and the render comparator itself. Evidence is pinned to
tool versions and font environment, including explicitly qualified reader and
renderer diagnostics. It does not establish arbitrary visual fidelity or PDF/UA.

Runtime receipts contain same-engine checks only. They neither run these external
tools nor gain a runtime external-oracle guarantee from passing a suite. Incremental
preserves prior source bytes: replacement is not secure deletion. Compact here
also is not certified secure redaction.

Next bounded package: P4a, native/WASM retained-session contract and artifact
verification. First inventory existing session APIs and installed target/tooling;
define ownership, limits, cancellation, stale snapshot and explicit disposal
behavior, then test the same admitted operations across native and WASM. Missing
WASM runtime/target is a concrete prerequisite to resolve before claiming parity.
Keep Oxpdf as a minimal acceptance consumer, without editor features or overrides.
