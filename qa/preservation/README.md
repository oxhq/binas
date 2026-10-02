# Frozen preservation sources rescued for the nontext gate

This directory contains original Binas QA sources, Apache-2.0; see `LICENSE`.
PDF source bytes were already tracked in the candidate. The runners, license,
manifest and six provenance records were copied from the original local Binas
working tree without modifying the oracle bytes. No third-party PDF/font/artwork
assets were imported. Run `../nontext-boundary/run.py`, not fixture-generation
commands, for the current required gate and external evidence destination.

| Rescued sources | Required use |
| --- | --- |
| `oracle.py`, `alpha_oracle.py`, `render_oracle.py`, `manifest.json` | Reorder inventory/control checks, normative alphabetic-label control, and all-page RGB comparison |
| `geometry_oracle.py` | Absolute page geometry plus reorder; also a geometry helper for the subsequent families |
| `flate_oracle.py` | Shared typed pypdf object/stream inventory, hashes and serialization helpers; its standalone Flate fixture/pass is not part of this gate |
| `checkbox_oracle.py`, checkbox provenance | Existing checkbox states, preserved APs and reader-warning controls |
| `flatten_oracle.py`, flatten provenance | Targeted flatten and shared exact incremental-xref proof/warning policy |
| `text_field_oracle.py`, text-field provenance | Frozen source/template generator equality, Tx AP-copy checks and helpers for duplication/deletion/images |
| `duplicate_oracle.py`, duplicate provenance | Adjacent page duplication, independent fields, labels, registration and pixels |
| `delete_oracle.py`, delete provenance | Page deletion, explicit local target remaps, registrations and pixels |
| `lossless_image_oracle.py`, image provenance | Exact samples/predictors/shared masks after required edit and independent RGB expectation |
| `import_oracle.py`, `import_render.py` | Existing two Apache-2.0 Type3 fixtures under crate tests; extraction/merge/sharing and rendering |
| `requirements.txt`, `LICENSE`, `.gitignore` | Exact dependency pin, retained license and exclusion of generated Python bytecode |

Every retained fixture hash is checked before running the public examples. Existing
provenance links source hashes, original generator hashes and current oracle hashes;
generation and historical license/content are not reasserted from engine receipts.
The runner stages unmodified oracle/provenance bytes and copies the modified public
examples for correct evidence hashes. Example changes only add explicit input/output
environment paths so the same probes can run against extracted crate consumers.
Large evidence, renderings and derived controls belong outside this checkout.
