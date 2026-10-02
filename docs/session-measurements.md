# Local retained-session measurements

P6 measures the native Rust `PdfSession` boundary in the local 0.2.0 candidate.
It compares selected reads and an equivalent four-edit Compact workflow against
reopening sessions, after correctness gates. It is neither a Bento comparison
nor a universal PDF benchmark. The source baseline is
`e0e8836324832fffdfde01774fd32adf478d27e1` plus the P5/P6 candidate changes;
`environment.json` records the exact working-tree inventory, example SHA-256,
lockfile SHA-256 and toolchain. The final release evidence records the committed
candidate separately. Measurements preceded that commit and do not alone verify
the packaged artifact.

## Reproduction and evidence

Run from the checkout with the installed Rust toolchain, PowerShell and Python
interpreter with the existing P3a pypdf/Pillow/Poppler dependencies:

```powershell
& qa/session-measure/run.ps1 -Evidence 'C:\path\to\scratch\p6-measure'
```

`-Python` can select that already-installed interpreter. The script builds with
`cargo build --offline --locked --release -p binas-pdf --example session_measure`
in an isolated target directory. The example takes runtime repository/evidence
paths; fixtures are not embedded into the production crate. No installation,
credentials, registry publication or remote documents are involved.

The recorded local run is
`C:\Users\garae\Documents\Codex\2026-10-01\task-8\p56\p6-measure`.
It contains `environment.json`, `binary.json`, `build.log`, `run.log`,
`correctness.json`, all raw nanosecond observations in `samples.json`, and
`summary.json` with sample count, minimum, nearest-rank p50/p95 and maximum.
`manifest.json` hashes durable evidence except the build target, itself and the
summary stdout log. Eight emitted PDFs and their independent P3a oracle reports
are retained outside the source tree. Independent diagnostic qualifications
described in [retained-ascii QA](../qa/retained-ascii/README.md) apply unchanged;
target appearance remains NotChecked.

The run used Windows 11 Pro x64 build 26200, AMD Ryzen 9 9950X3D (16 physical
cores, 32 logical processors), 134,907,559,936 installed RAM bytes, Balanced power
plan, rustc/cargo 1.95.0, MSVC target and LLVM 22.1.2. This installed-RAM quantity
is host metadata, not measured workload memory. Other local candidate gates may
have run concurrently; CPU affinity, power state and system background activity
were not controlled.

## Correctness and workload

Four original Apache-2.0 fixtures from P3a/P3b are used: retained raw ASCII and
explicit-CMap fonted text, each plain/Flate, three pages and about 4–6 KiB.
Manifest provenance and exact source SHA-256s are retained. Selected page zero
must yield `ALPHA`, `KEEP`. Fonted selected reads must be warning-free and raw
session preparation must refuse fonted input with `unsupported_feature`, keeping
snapshot and text unchanged. Raw selected extraction retains its actual warning
count in `correctness.json`; raw fontless text is not complete font-aware text
or target visual evidence.

For each raw storage shape and both writer modes, preparation must leave the
live text unchanged, acceptance must identify only object 4, a consumed handle
must be stale, repeated independently opened sessions must yield identical
output bytes, seven writer checks must pass, and reopening the output must yield
`OMEGA`, `KEEP`. Close must reject subsequent reads. For each shape the sequence
`ALPHA → OMEGA → SIGMA → DELTA → OMEGA` uses Compact; retained and reopened
routes must finish with identical bytes and the expected selected text. All
eight single/sequence outputs pass the existing independent P3a preservation,
strict parser, geometry and untouched-page renderer oracle. There is no engine
change in this harness and no new fonted session-edit support.

Timing follows those checks, with three unreported warmups and 21 recorded
samples per case. All timings are `Instant` elapsed wall time:

* `open_ns` measures a new session's input admission, hash and parse. Its owned
  input copy is prepared outside that timer; this is a cold **session** in an
  already running process, not cold OS/file-cache/process startup.
* `first_read_ns` measures the first selected read after open. Warm read batches
  repeat 32 reads on that same session after its first read, then divide batch
  duration by 32. Reopen read batches open, read and drop 32 sessions and include
  a fresh input allocation/copy for each. This is the actual lifecycle comparison,
  not an isolated extraction-speed comparison; read phases use fixed warm-then-
  reopen order.
* Raw preparation includes plan, unpublished commit, checked export and receipt
  construction. Acceptance includes bound request/evidence validation and
  publication. Export separately includes writing and same-engine reparse/checks.
  Public API atomic phases are not subdivided into hypothetical CPU-only work or
  independent-validator costs. The independent Python oracle runs outside timers.
* Two-session batches execute two distinct owners/contexts, each doing 32 reads.
  Both serial and two-thread cases include opening, first decode and drop;
  parallel timings also include thread spawn/join. They share immutable fixture
  bytes, not retained graphs. Four-edit sequence timings include initial open,
  preparation/acceptance, final checked export and result copying. The reopen
  path also exports/reopens between edits; retained avoids those three exports.
  This is equivalent final behavior with different lifecycle costs, not equal
  numbers of writer operations. Batch route order alternates by sample.

Each session uses an output-capacity ceiling of 1 MiB and logical operation
limits of 16 MiB live reservations and 256 Mi units of cumulative work, alongside
other default PDF limits. These are admission counters, not heap or CPU measures.

## Recorded results

Times below are microseconds, p50 / p95. Read batch values are **per read**;
the other phase values are per operation.

| Fixture | Open | First read | Warm retained read | Reopen + read + drop |
| --- | ---: | ---: | ---: | ---: |
| Raw plain | 22.2 / 25.0 | 4.1 / 14.5 | 2.54 / 2.68 | 27.32 / 28.30 |
| Raw Flate | 22.4 / 27.7 | 26.4 / 36.5 | 2.58 / 2.97 | 36.78 / 41.50 |
| Fonted plain | 22.0 / 28.3 | 54.7 / 64.5 | 52.50 / 63.50 | 78.72 / 92.71 |
| Fonted Flate | 22.0 / 25.9 | 63.9 / 73.0 | 51.93 / 52.69 | 89.48 / 93.33 |

| Raw fixture and writer | Prepare with checks | Accept/validation | Export with checks |
| --- | ---: | ---: | ---: |
| Plain Incremental | 121.3 / 139.3 | 21.2 / 29.8 | 47.2 / 53.0 |
| Plain Compact | 139.6 / 155.7 | 22.2 / 33.1 | 82.9 / 112.7 |
| Flate Incremental | 239.0 / 281.7 | 21.7 / 37.8 | 47.2 / 55.4 |
| Flate Compact | 276.8 / 315.6 | 31.4 / 41.4 | 83.6 / 94.9 |

The raw Flate batch totals are microseconds per full batch:

| Workload | Retained p50 / p95 | Reopen p50 / p95 |
| --- | ---: | ---: |
| Two serial independent sessions × 32 reads | 251.7 / 278.9 | 1967.5 / 2038.6 |
| Two concurrent independent sessions × 32 reads | 319.0 / 347.7 | 1270.5 / 1578.7 |
| Four Compact edits, one final output | 1055.2 / 1173.0 | 1377.2 / 1497.2 |

Thread overhead exceeds the tiny retained serial read batch here. Fonted reads
remain much more expensive than raw warm reads. Preparation and checked exports
are material parts of total edit work; session retention does not remove them.
The full min/max variation is available in `summary.json`, including a 195.7 μs
plain Incremental preparation maximum and a 1704.7 μs reopened four-edit maximum.

## Scope relative to the original plan

This fulfills the original P6 requirement for measured same-correctness initial/
warm and serial/concurrent behavior on named local cases. The authorized release
slice uses native retained raw editing and native fonted selected reads; JavaScript
session performance, large documents, many producers and secure/redaction/OCR
workloads are outside this batch. Session reopen uses the new candidate API on
both routes: it is not a measured 0.1.1-versus-0.2.0 comparison.

Actual peak heap/process memory was not measured: the environment has no scoped
allocator instrumentation and whole-process working-set readings would mix
runtime, parser caches, other threads and output ownership. Original P4 actual-
memory proof therefore remains open and no total heap/CPU budget is advertised.
No Bento executable or equivalent competitive benchmark was run. Package,
consumer and migration evidence is reported separately by the P6 artifact review.
Repeated-read medians apply only to these small fixtures on this host; this batch
establishes neither general speedup nor latency/service guarantees.
