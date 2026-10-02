# Observed native and WASM peak memory

This gate measures actual memory separately from session resource budgets. It
does not raise any engine limit. The fixed textual cohort uses a public Rust
`PdfSession` example and the generated public JS `BinasPdfSession` binding. The
native nontext slice executes the public workflows from
`tests/nontext_boundary.rs`; its observations cover those entire small workflows,
not every possible input for those capabilities.

## Reproduction and evidence

Build and run from the checkout using the installed Windows tools. `ARTIFACTS`
contains freshly generated `pkg-node` and `pkg-web`; keep `EVIDENCE` outside the
repository. No dependency installation is required by these probes.

```powershell
cargo build --offline --locked --release -p binas-pdf --example boundary_peak_memory --target-dir EVIDENCE/target
python -B qa/peak-memory/run.py --repo REPO --artifacts ARTIFACTS --binary EVIDENCE/target/release/examples/boundary_peak_memory.exe --evidence EVIDENCE
node qa/peak-memory/browser.mjs ARTIFACTS EVIDENCE 3
cargo test --offline --locked -p binas-pdf --test nontext_boundary --no-run --target-dir EVIDENCE/target
python -B qa/peak-memory/nontext.py --repo REPO --binary EVIDENCE/target/debug/deps/nontext_boundary-HASH.exe --page-binary EVIDENCE/target/release/examples/boundary_peak_memory.exe --page-golden B_EVIDENCE/import-shared-duplicate.pdf --evidence EVIDENCE/nontext
```

`BINAS_CHROMIUM` and `BINAS_CHROMEDRIVER` can override the browser/driver paths.
The browser adapter uses the already installed Chromium and WebDriver, served
over loopback. The Python sampler uses only its standard library and Windows
`GetProcessMemoryInfo`.

The authoritative run emits `cases.json`, `environment.json`, `samples.json`,
`summary.json`, `browser.json` and `nontext/nontext.json`. It records fixture,
binary, WASM, lock, harness and engine/binding source hashes, the checkout SHA,
tool versions, commands, samples, failures and denominators. Exports remain next
to the observations, with SHA-256 parity across native/Node/Chromium and retained/
reopen variants. A refused operation is a successful probe only when its expected
stable error code and unchanged revision are checked. Failed probe attempts are
kept separately and never counted as passed memory observations.

## Cohort and denominator

The textual matrix has **18 cases per runtime**, **three observations per case**:
108 fresh native/Node processes and 54 Chromium document instances. Plain and
Flate fixtures exercise 32 selected reads with retained sessions or reopening,
and four Compact ASCII edits with retention or export/reopen between edits. The
two edit routes must emit exactly the same final bytes. Reopening writes three
intermediate exports; retention writes only the final export, so this is a
workflow comparison with different amounts of serialization.

The boundary cases include admitted input exactly 8 MiB, admitted selected Flate
expansion exactly 16 MiB, and refusal at 8 MiB+1 input and 16 MiB+1 decoded bytes.
The output growth case replaces `ALPHA` with 1 MiB of ASCII and exports Compact.
The output refusal uses the 8 MiB padded input, an 8 MiB-4 ASCII replacement and
Incremental mode: the resulting output exceeds the fixed 16 MiB output limit.
Its error diagnostic must identify `output exceeds max_output_bytes` and its
revision remains zero. **Both output cases use a 48 MiB caller logical live
budget**, allowing the refusal case to reach that output check; the other textual cases use 24 MiB, except the deliberate
one-byte live-budget refusal. All use 256 Mi logical work units. Additional
refusals cover an invalid header, unsupported fonted raw edit and cancellation
already signaled before a read. These inputs are original synthetic fixtures;
generated padding/Flate boundary data is produced before the observed process.

The nontext denominator is six workflows times three fresh native processes:
annotation create/contents/remove, form create/value/remove and registration,
image resource query/subtype refusal, invalid selections/geometry refusal, and
raw RGB image replacement/refusals, and page extraction/import with order and
duplication `[1, 0, 1]`. The first five use a debug Rust test executable; page
assembly uses the release public example and frozen shared Type3 fixture. Its
source remains two pages, its candidate reparses as three pages, and its bytes
must match B's independently qualified `import-shared-duplicate.pdf` export.
All use the existing licensed preservation cohort. Default native engine limits differ
from the fixed WASM/text-session limits. Do not compare their process sizes as
if they were the same build or limits. Nontext WASM exposure is absent and no
memory observation is claimed for it.

## What each metric means

| Metric | Observation and limits |
| --- | --- |
| Native/Node `PeakWorkingSetSize` | Windows process lifetime peak resident working set. The child waits before input and after work so the parent reads the OS-maintained peak before exit; it captures short peaks missed by sampling. Includes runtime startup, caller buffers, module/JIT, engine and report/file-I/O overhead. It is not an engine allocation count. |
| Native/Node working set and private bytes | Parent snapshots requested every 2 ms, with timestamps and sample counts. Actual scheduling varies. Sampled maxima are lower bounds on intermediate peaks. `PrivateUsage` is committed private memory, not resident RSS. Peak pagefile usage is recorded separately and is not relabeled as private peak. |
| WASM linear memory | Exported `WebAssembly.Memory.buffer.byteLength`, observed after module initialization and between phases, including after close/free. These modules use ordinary growable non-shrinking memory, so final capacity is its exact linear-memory high-water capacity for this instance. It includes allocator free capacity and cannot be interpreted as live Rust payloads or all process memory. No JS view is retained across growth. |
| Node JS heap | `process.memoryUsage()` at phase boundaries gives heap used/total, external and ArrayBuffer observations. The maximum among these observations is a phase-sampled lower bound, not a JS heap peak inside synchronous calls. Node RSS is also observed; process memory and linear memory overlap and must not be added. |
| Chromium JS heap | `performance.memory` observations with the precise-memory-info flag are diagnostic heap snapshots for a reused browser and fresh document/module instance. They are not isolated per-engine peak allocations. Renderer/browser/GPU process RSS and private memory are not measured. |
| Logical budgets | Reservations and cumulative work counters at open/close, with zero managed live bytes after close. They are not bounds on actual RSS, JS heap or WASM linear memory. Returning output also leaves caller-owned bytes outside managed session counters. |

No forced GC is performed. Closing/freeing sessions releases their managed
reservations; WASM linear capacity and native allocator/runtime caches can remain
high. The parent sampler's memory is excluded from the child, but child
instrumentation, report serialization, fixture ownership and export I/O remain
included. The readiness baseline is before file input, after Node/WASM module
load; subtraction from that baseline does not isolate Binas memory and is not
presented as such. Native inputs retain the caller's original buffer while
cloning/moving input into each session. JS input/output crosses the binding by
copy, including a transfer before over-limit constructor refusal. The harness
records known caller moves/copies and JS transfer byte counts, not an exhaustive
allocation or copy census.

## Results and proof boundary

The checked run is recorded outside the repository in the task evidence index.
Its numeric observations and exact source/artifact identities are authoritative;
the repository documents the method and scope so future reruns do not inherit
this host's numbers as a portable guarantee. Actual process and WASM linear
observations are reported separately from the logical decoded limit.
The output-refusal case can allocate substantial temporary
buffers before failing the writer's 16 MiB output check.

This gate proves measured behavior and expected refusals for the named licensed
and synthetic cohort on one Windows/MSVC, Node and Chromium environment. It does
not establish a worst-case admitted envelope for all PDFs, total CPU, full JS
heap peaks, hosted CI, other operating systems, malicious-input exhaustiveness,
worker recovery or browser mid-call cancellation. The browser cancellation case
is synchronous pre-call cooperative cancellation. A hard process/worker memory
cap and terminating a blocked worker remain host responsibilities and are not
implemented by this probe.
