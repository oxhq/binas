# Local h4x boundary progress: retained text sessions

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
| P4a | Local candidate: bounded raw sessions, ownership/disposal, native and real Chromium Rust parity, public JS exports in Node/Chromium | Cooperative pre-call cancellation; no worker termination/recovery or process peak-memory claim |
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

The bounded session contract is in [pdf-sessions.md](pdf-sessions.md). Its raw
operation remains the existing P2b capability. The fonted P3b cohort runs through
the shared Rust revision API in the real browser runtime, without introducing a
fonted JS capability. Both runtime routes compare to frozen independently checked
P3 output bytes. Packaging/migration evidence is separate from runtime receipts.

Remaining P4 work is a user-scoped support decision: this synchronous single-thread
profile can close without promising worker cancellation/recovery or total memory
control. Implement those only if selected for release support. P5 is one bounded
producer-diverse held-out text qualification cohort with licensed provenance and
independent effect/refusal checks. P6 retains measured performance, version/support
decision, final release artefact review and authorized public installation proof.
Keep Oxpdf as a minimal acceptance consumer; expand neither editor nor UI.
