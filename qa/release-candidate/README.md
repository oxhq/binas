# Local release qualification

`package.py` packages core, PDF and CLI using default Cargo verification. It
accepts only a committed, clean candidate, creates disposable local source
staging, resolves unpublished siblings with patches to already extracted archives,
and verifies source hashes, licenses, package inclusion and consumer metadata.
Staging resolver metadata may change; the candidate lock is checked unchanged.
No publishing, Actions dispatch, installation or credentials are used.

```powershell
python -B qa/release-candidate/package.py --source . --out C:\scratch\packages --target C:\scratch\target --oxpdf-source C:\scratch\existing-p1-oxpdf
```

Outputs include three `.crate` archives, exact file lists/hashes, command logs,
metadata, a real selected/session/legacy Rust consumer, an extracted CLI run and
an optional seven-case Oxpdf page-text canary copied from the existing P1 source.
Its source hashes and origins are recorded; the original Oxpdf checkout is untouched.
The default MSRV gate uses an **already installed** 1.88.0 toolchain. Pass
`--msrv ''` only when running a separate stable-only package gate, and do not count
that as an MSRV pass.

Libraries are consumed from archives, with explicit artifact-only sibling patches.
This is local artifact proof, not crates.io installation. Archive-local repository
tests are not distributed; private cfg(test) code needs checkout fixtures. See
`docs/release-surface.md` for migration, support and publication approval scope.
