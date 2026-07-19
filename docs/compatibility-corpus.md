# Licensed Compatibility Corpus Gate

The PDFs under `testdata/pdf` are frozen for local unit boundaries only.  Their
source and license are unverified, so they are not compatibility, adversarial,
or release evidence.

The `external_compatibility_corpus` integration test accepts a separately
provisioned, licensed corpus.  It never downloads fixtures: the corpus owner
must provide the bytes and manifest to the test runner.

## Manifest

Set `BINAS_COMPAT_CORPUS_MANIFEST` to a JSON file outside or inside the checkout.
The manifest's `corpus_root` is relative to the manifest file, which makes a
mounted corpus portable while preventing fixture paths from escaping that root.

```json
{
  "version": 1,
  "corpus_root": "pdf",
  "fixture_count": 2,
  "coverage_status": "licensed_compatibility_and_adversarial",
  "fixtures": [
    {
      "id": "producer-a-basic",
      "path": "producer-a-basic.pdf",
      "sha256": "lowercase-64-character-sha256",
      "source": "upstream URL, archive identifier, or procurement record",
      "license": "SPDX identifier or reviewed license reference",
      "roles": ["compatibility", "metadata"],
      "parse": "opens",
      "expected_pages": 1
    },
    {
      "id": "malformed-xref",
      "path": "malformed-xref.pdf",
      "sha256": "lowercase-64-character-sha256",
      "source": "upstream URL, archive identifier, or procurement record",
      "license": "SPDX identifier or reviewed license reference",
      "roles": ["adversarial", "xref"],
      "parse": "rejects",
      "expected_error_code": "invalid_syntax"
    }
  ]
}
```

Run the optional gate locally with:

```powershell
$env:BINAS_COMPAT_CORPUS_MANIFEST = 'C:\corpora\binas\manifest.json'
cargo test -p binas-pdf --test external_compatibility_corpus
```

Set `BINAS_REQUIRE_COMPAT_CORPUS=1` only on a runner where that corpus is
available.  It converts an absent manifest into a failure, so a release gate
cannot silently skip the corpus.  This gate proves Binas parse/inspect/validate
behavior and fixture integrity; independent interoperability runs and sustained
fuzz/performance measurements remain separate requirements.
