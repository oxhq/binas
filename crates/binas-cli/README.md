# binas-cli

Command-line interface for the Binas PDF inspection and verified rewrite engine. The package installs a `binas` executable.

```powershell
cargo install binas-cli
binas inspect C:\path\file.pdf --json
```

Engine behavior lives in [`binas-pdf`](https://crates.io/crates/binas-pdf); this crate owns argument parsing, filesystem I/O, and JSON presentation.

Licensed under Apache-2.0.
