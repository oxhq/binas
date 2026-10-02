# Public nontext boundary gate

This gate qualifies a finite original Binas synthetic cohort, Apache-2.0.
It uses public Rust APIs, strict pypdf 6.10.0 object/sample checks and Poppler
RGB144/CropBox rendering. It does not establish arbitrary PDF compatibility,
standards compliance, hosted CI, WASM parity or release readiness.

Run with the installed Python containing the pinned dependencies; no installation
or network access is part of this gate:

```powershell
python qa/nontext-boundary/run.py --output C:/external/evidence/nontext --pdftoppm C:/installed/poppler/pdftoppm.exe
```

Evidence output must be outside the checkout. The runner stages exact oracle,
fixture, provenance and example bytes there and always creates fresh engine
outputs before checking them. Missing inputs, hashes, runtime dependencies,
outputs, reader diagnostics or failed comparisons fail the command. Optimized
Python is rejected because the rescued independent oracles use assertions.

To repeat against extracted crate archives, prepare an external Cargo consumer
with `binas-core` and `binas-pdf` dependencies pointing to those archives. Copy
the nine `preservation_*.rs` examples named in `run.py` into its `examples/` and
`nontext_boundary.rs` into its `tests/`. Generate its lockfile offline, then add
`--consumer-manifest C:/external/consumer/Cargo.toml` to the same command.
The runner sets fixture paths from this checkout and directs all output externally.
Cargo runs offline/locked in both modes. Consumer dependency identity is checked
by the packaging gate, not inferred by this runner.

## Qualified cases

| Family | Input and independent evidence | Limits |
| --- | --- | --- |
| Retained pages | Full permutation and inherited geometry on reorder/alpha fixtures; adjacent duplication and page deletion on frozen text-field fixture; Incremental/Compact full original inventory and all output pages | Specific labels/navigation/forms and explicit delete policy; detached bytes persist; no secure erasure |
| Legacy pages | Extract with repeated occurrences, merge two Type3 fixtures and extract the merged result; four outputs, exact glyph/resource sharing, content and all-page pixels | Two frozen original Type3 inputs; no general import/reconciliation promise |
| Retained forms | Checkbox existing states; targeted appearance flatten; fixed-template text fill with owner-local AP copies; both modes, complete inventory, samples and all-page pixels | Existing supported appearances/templates; no arbitrary appearance generation, shaping or layout |
| Legacy lifecycle | Create/remove Text annotation and Tx field on frozen text-field source; original object/encoded-stream preservation, requested registrations and AP reachability | Created appearance visibly changes only target rectangle and leaves the other page exact; icon/font style lacks an independent pixel golden |
| Annotation Contents | Legacy and retained I/C change one appearance-free Text note; full untouched inventory and all-page pixels against pypdf-authored expectation | Explicit same-size shape control converts source Link18 to Text; all other original bytes/objects stay exact. Existing AP mutation and Link subtype are refused |
| Image replacement | Tiny unmasked RGB8 image13, Raw2x2 samples `[20,80,180]` repeated four times; full inventory, every other encoded stream, exact samples and pypdf-authored pixel golden | One raw RGB shape only; masked image7 and wrong sample count refused; JPEG/JPX/other layouts remain outside this independent slice |
| Lossless image storage | RGB/Gray predictors on frozen source after a required text edit, retained masks/shared nested resources and exact samples, both modes and all-page pixels | Declared direct Flate8bit Gray/RGB layouts only; final size and payload savings are distinct |

`nontext_boundary.rs` has five public workflows. Invalid selections/geometry,
appearance-bearing mutation, non-Text mutation, wrong image sample count and
masked-image policy are explicit rejection assertions. Rejections do not count
as successful edits. Memory sampling may set `BINAS_NONTEXT_MEMORY_HOLD=1` and
run one test with `--nocapture --test-threads=1`: READY/DONE each wait for stdin
newline. This handshake is measurement coordination, not an engine change.

The independent controls include unchanged sources lacking requested creation or
image effects, wrong Contents/sample bytes and collateral sibling/metadata changes.
Every semantic corruption is independently parseable. The exact known pypdf sparse
incremental-xref diagnostic is accepted only after independent source-prefix,
Prev, xref-offset and object-header proof; unknown/misplaced warnings fail.
Poppler's two pinned ambient display-font diagnostics are recorded where present;
all other diagnostics fail in the new oracle. No warning-free renderer claim follows.

`summary.json` records each command/log/hash, source HEAD/status before and after,
runtime inputs and artifact inventory. HEAD and all protected source inputs must
stay identical during the gate. A dirty development gate is identified explicitly;
the final clean checkout/package rerun is a separate release evidence boundary.

The first complete development run passed 25 commands, five public tests,
31 distinct engine artifacts (including the required image-edit baseline),
72 semantic corruption controls, 30 warning/xref controls and 75 positive page
comparisons: 73 exact RGB comparisons plus two creation-effect bounds checks.
Counts describe unique family/control definitions, not repeated executions of
the same control in the two reorder writer checks. The source SHA, output hashes
and current final result belong to the external receipts, not this historical note.
