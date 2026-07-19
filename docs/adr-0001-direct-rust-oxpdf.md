# ADR 0001: Direct Rust OXPDF integration

Status: accepted

OXPDF will be rewritten in Rust after the Binas Rust engine is complete and will depend directly on `binas-pdf`. No temporary C ABI or CLI subprocess bridge will be built. The Go OXPDF implementation remains independent rollback/reference material until the Rust rewrite passes its engine-facing suite.
