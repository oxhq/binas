"""Narrow Table 159 check, explicitly separate from pypdf display labels."""
import argparse
import io
import json
from pathlib import Path

import oracle

SOURCE = "https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf"
VERIFIED_MIRROR = "https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf"


def normative_labels(reader):
    nums = reader.trailer["/Root"]["/PageLabels"]["/Nums"]
    entries = {int(nums[i]): nums[i + 1].get_object() for i in range(0, len(nums), 2)}
    assert 0 in entries
    result = []
    for page in range(len(reader.pages)):
        start = max(index for index in entries if index <= page)
        entry = entries[start]
        prefix = str(entry.get("/P", ""))
        if "/S" not in entry:
            result.append(prefix)
            continue
        assert entry["/S"] == "/A", "This oracle slice only implements uppercase alphabetic labels"
        number = int(entry.get("/St", 1)) + page - start
        assert 1 <= number <= 100, "Synthetic fixture bound"
        # ISO 32000-1 Table 159 uses repeated letters after Z, not base-26 words.
        result.append(prefix + chr(ord("A") + (number - 1) % 26) * ((number - 1) // 26 + 1))
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["source", "verify"])
    parser.add_argument("--candidate", type=Path)
    args = parser.parse_args()
    source = (oracle.ROOT / "fixtures/alpha-boundary.pdf").read_bytes()
    manifest = json.loads((oracle.ROOT / "manifest.json").read_text())
    assert oracle.sha(source) == next(item["sha256"] for item in manifest["required"] if item["path"] == "fixtures/alpha-boundary.pdf")
    reader = oracle.pypdf.PdfReader(io.BytesIO(source), strict=True)
    assert normative_labels(reader) == ["Z", "AA", "BB"]
    assert reader.page_labels == ["Z", "AA", "AB"], "Pinned pypdf disagreement changed; review the oracle"
    report = {"source_sha256": oracle.sha(source), "normative_source": SOURCE, "table": 159, "pypdf": oracle.pypdf.__version__, "source_normative_labels": normative_labels(reader), "source_pypdf_labels": reader.page_labels, "pypdf_disagreement": "28 is AB in pypdf 6.10.0, BB in Table 159; no pypdf correctness claim for alphabetic label display", "oracle_sha256": oracle.sha(Path(__file__).read_bytes()), "shared_oracle_sha256": oracle.sha(Path(oracle.__file__).read_bytes())}
    name = "alpha-source"
    report["verified_official_mirror"] = VERIFIED_MIRROR
    report["normative_pdf_page_index"] = 382
    if args.command == "verify":
        assert args.candidate is not None
        candidate = args.candidate.read_bytes()
        output = oracle.pypdf.PdfReader(io.BytesIO(candidate), strict=True)
        assert normative_labels(output) == ["BB", "Z", "AA"]
        oracle.verify(source, candidate, [2, 0, 1], normative_labels)
        report.update(candidate_sha256=oracle.sha(candidate), candidate_normative_labels=normative_labels(output), candidate_pypdf_labels=output.page_labels, result="source_linked_normative_and_nonlabel_preservation_checks_passed")
        name = args.candidate.stem
    else:
        report["result"] = "source_checked_with_explicit_reader_disagreement"
    report["reader_warnings"] = sorted(set(oracle.WARNINGS))
    (oracle.ROOT / f"evidence/{name}.json").write_text(json.dumps(report, indent=2)+"\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
