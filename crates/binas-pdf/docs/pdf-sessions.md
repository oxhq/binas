# Retained PDF sessions

This checkout adds `PdfSession` and the synchronous `BinasPdfSession` JavaScript
binding. These APIs are not present in the published `binas-pdf 0.1.1`. The 0.2.0
local release candidate proposes synchronous native, Node and Chromium support.
Publication and approval of that support profile remain pending.

The session retains one source and current revision. Selected page reads return
`TextExtraction`; any warning can mean incomplete extraction. Preparation uses
the existing conditional `pdf.text.retained_ascii` contract version 1: one whole
ASCII token in a uniquely owned supported content stream, without font fallback.
Only Incremental and Compact exports are admitted by this contract. Font-aware
page edits remain a separate Rust `PdfRevision` operation; they are not a new
JavaScript session capability. Text replacement is not secure redaction.

## Ownership and lifecycle

`PdfSession::open(Vec<u8>, EngineConfig, OperationContext)` takes ownership of
input without another retained payload copy. A session cannot be cloned. Its
opaque `PdfSessionPlanHandle` contains ownership, snapshot and a monotonic
generation; handle clones retain no PDF data. A handle from another session is
foreign even when input bytes and snapshot match. Source identity is the original
SHA-256 and revision advances once at successful acceptance.

At most one prepared candidate is held. `prepare_text_edit` runs the existing
plan/commit/checked-export path on an unpublished candidate. Live reads still see
the prior revision. `accept_text_edit` publishes that candidate only after its
bound request, contract and required checks pass. A successful handle becomes
stale; an explicitly released or consumed rejected handle becomes released.
`release_plan` discards an unaccepted candidate. Freeing a JavaScript handle only
frees that token: call `releasePlan` or close its owner to release the candidate.

`close` is idempotent and works after cancellation or exhausted budgets. It drops
the current graph, caches and pending candidate, releasing all session-owned
reservations, even if tokens remain alive. Native Drop does the same. JavaScript
callers should call `close()` and then the generated `free()`; `free()` drops the
Rust owner, while garbage collection timing is not a deterministic disposal API.
Closed sessions reject graph operations with `session_closed`. Snapshot identity
and budget counters can be observed after cancellation; snapshots reject closure.

## Accounting and retry

The operation budget reserves exact original input length until close/drop. Before
preparing or exporting it reserves the configured **maximum output capacity**,
not the observed eventual output length. A pending candidate retains that capacity
until acceptance/release/close. Export releases its reservation when it returns;
the caller owns the returned Source independently. A simultaneous export while a
candidate is pending needs another output-capacity reservation.

Work units are cumulative: opening charges input length, a selected read charges
one admission unit, acceptance charges two admission/validation units, and the
existing planner/writer add their documented traversal and byte work. Failed
attempts do not refund work already charged. ResourceBudget clones share counters
and limits. These logical units do not measure CPU time. Input size is checked
before hashing, charging or parsing an oversized input.

PDF decode cache/live/work limits are separate and cumulative across the retained
document's internal clones. These counters exclude parser/object maps, native
allocator overhead, codec scratch, returned output allocations and JavaScript
copies. They are not a bound on total process or WASM heap memory.

Cancellation is sticky and shared by token clones. A cancellation or work-budget
failure before acceptance admission preserves the pending plan. Once acceptance
starts, a rejection consumes it without publishing a revision. `renew_cancellation`
creates a fresh operation token with the same ResourceBudget and retained decode
work; old token clones remain cancelled. Retry after consumed rejection requires
preparing again. Exhausted cumulative work requires a new session/context; renewing
only cancellation does not refill either work budget.

The synchronous JS methods do not yield the event loop. `cancel()` is observed at
subsequent cooperative checkpoints; this does not implement mid-call browser
cancellation, worker termination or automatic recovery. Native cancellation from
another thread is cooperative, and legacy codecs/parser loops are not all
interruptible. A signal after the final publication checkpoint can arrive after
successful acceptance.

## JavaScript calls and errors

```javascript
const session = new BinasPdfSession(bytes, 24 * 1024 * 1024, 256 * 1024 * 1024);
let plan;
try {
  const extraction = JSON.parse(session.extractPageText(0));
  if (extraction.warnings.length) throw new Error('incomplete selected extraction');
  plan = session.prepareTextEdit('ALPHA', 'OMEGA', 0, 'compact', 1);
  const receipt = JSON.parse(session.acceptTextEdit(plan));
  const output = session.export('compact'); // separately owned Uint8Array
} finally {
  session.close();
  plan?.free();
  session.free();
}
```

The binding checks an 8 MiB retained input ceiling and has 16 MiB decoded-stream,
total decoded, decode cache/live and output ceilings. Its caller supplies the
logical live/work budget; the example budget allows one 16 MiB candidate plus
input and planned replacement reservations. The JS boundary already copies input
into WASM before the retained input check, and returned bytes copy into a separate
Uint8Array. No zero-copy or peak-memory claim follows.

Methods return a snapshot/extraction/receipt JSON string, an opaque plan, bytes or
void as appropriate. Failures throw `{code, message}`; branch on stable `code`:
`session_closed`, `foreign_plan`, `stale_plan`, `plan_released`, `plan_pending`,
`resource_limit`, `cancelled`, `contract_version`, `unsupported_writer_mode`, or
the existing PDF error code. Rust preserves typed underlying Context/PDF/Text
errors. Receipt JSON identifies contract/engine, input/output and before/after,
with its request, writer mode, required checks, changed objects, effective PDF and
context limits, eighteen mandatory same-engine checks and three explicit `not_checked`
scope entries. It is not independently verified visual/accessibility evidence.

## Local verification profile

`bindings/binas-pdf-wasm/tests/session_contract.rs` runs the same consumer tests
natively and in a real single-thread Chromium WASM runtime. Plain/Flate and both
writers must equal eight frozen outputs from the independently checked raw and
explicit-CMap text cohorts. The fonted parity test uses the shared Rust API and
checks that raw session preparation refuses it.

`qa/session-runtime/run.mjs` exercises generated public JS classes in Node and
Chromium with the same test function: repetition, ownership, closure, cancellation,
typed refusals, byte equality and input/decode limit failures. No benchmark,
concurrent worker recovery, universal PDF/UA claim or producer-wide compatibility
is established by these finite fixtures. Independent oracles remain separate
from runtime receipts; new runtime byte parity reuses frozen checked outputs.
