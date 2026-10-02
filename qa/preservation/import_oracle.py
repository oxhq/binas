"""Independent pypdf checks for original frozen Binas import fixtures.

No Binas report is used as a preservation oracle. These structural/content
checks are limited to the two generated Type3 fixtures, not broad PDF fidelity.
"""
import argparse
import hashlib
import io
import json
import logging
import platform
from pathlib import Path

import pypdf
from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, StreamObject

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
FIXTURES = REPO / "crates/binas-pdf/tests/fixtures/preservation"
EVIDENCE = ROOT / "evidence"
SOURCE_HASHES = {
    "type3-shared.pdf": "42370c67047e1878374c8e81a3c5418c3000515fcdf5bd9e80cc9435ab292445",
    "type3-separate.pdf": "12f26ccd42875d258450a9b69ba80146fa70c25f175af68819d41549bc16aafe",
}
S, D = SOURCE_HASHES
CASES = {
    "import-shared-duplicate": [(S, 1), (S, 0), (S, 1)],
    "import-separate-duplicate": [(D, 1), (D, 0), (D, 1)],
    "import-merged": [(S, 0), (S, 1), (D, 0), (D, 1)],
    "import-merged-extracted": [(D, 1), (S, 0), (D, 0), (S, 1), (D, 1)],
}
WARNINGS = []


class Capture(logging.Handler):
    def emit(self, record):
        if record.levelno >= logging.WARNING:
            WARNINGS.append(record.getMessage())


logging.getLogger("pypdf").addHandler(Capture())


def sha(data):
    return hashlib.sha256(data).hexdigest()


def checker():
    if not __debug__:
        raise RuntimeError("Run without -O: assertions are required")
    assert pypdf.__version__ == "6.10.0", "Install pinned requirements.txt"
    return {
        "name": "pypdf", "version": pypdf.__version__,
        "python": platform.python_version(),
        "checker_sha256": sha(Path(__file__).read_bytes()),
        "pypdf_module_sha256": sha(Path(pypdf.__file__).read_bytes()),
    }


def reader(path):
    return pypdf.PdfReader(io.BytesIO(path.read_bytes()), strict=True)


def canonical(value, ancestors=()):
    """Compare decoded bytes and semantic dictionaries, independent of IDs/xrefs."""
    if isinstance(value, IndirectObject):
        key = (value.idnum, value.generation)
        assert key not in ancestors, "unexpected cycle in frozen resource fixture"
        return canonical(value.get_object(), ancestors + (key,))
    if isinstance(value, StreamObject):
        return {
            "decoded_sha256": sha(value.get_data()),
            "dictionary": {
                str(key): canonical(item, ancestors)
                for key, item in sorted(value.items())
                if key not in ("/Length", "/Filter", "/DecodeParms")
            },
        }
    if isinstance(value, DictionaryObject):
        return {str(key): canonical(item, ancestors) for key, item in sorted(value.items())}
    if isinstance(value, (ArrayObject, list, tuple)):
        return [canonical(item, ancestors) for item in value]
    if isinstance(value, bytes):
        return {"bytes": value.hex()}
    if isinstance(value, str):
        return str(value)
    if value is None or isinstance(value, (bool, int, float)):
        return value
    raise AssertionError(f"unhandled fixture object type {type(value).__name__}")


def geometry(page):
    return {
        name: [float(item) for item in getattr(page, name)]
        for name in ("mediabox", "cropbox", "bleedbox", "trimbox", "artbox")
    } | {"rotation": page.rotation}


def identities(page):
    resources = page["/Resources"]
    fonts = resources["/Font"]
    font = fonts["/F1"]
    values = {
        "resources": page.raw_get("/Resources"),
        "font": fonts.raw_get("/F1"),
        "glyph": font["/CharProcs"].raw_get("/A"),
        "content": page.raw_get("/Contents"),
    }
    assert all(isinstance(value, IndirectObject) for value in values.values())
    return {name: (value.idnum, value.generation) for name, value in values.items()}


def inventory(page):
    content = page.get_contents()
    assert content is not None
    return {
        "resources": canonical(page.raw_get("/Resources")),
        "decoded_content_sha256": sha(content.get_data()),
        "text": page.extract_text(),
        "geometry": geometry(page),
    }


def sources():
    manifest = json.loads((FIXTURES / "manifest.json").read_text(encoding="utf-8"))
    entries = {entry["path"]: entry for entry in manifest["fixtures"]}
    loaded = {}
    for name, expected_hash in SOURCE_HASHES.items():
        entry = entries[name]
        assert entry["mandatory"] and entry["origin"] == "original_generated"
        assert entry["license"] == "Apache-2.0"
        assert entry["sha256"] == expected_hash == sha((FIXTURES / name).read_bytes())
        source = reader(FIXTURES / name)
        assert len(source.pages) == 2
        assert [page.extract_text() for page in source.pages] == ["A", "AA"]
        loaded[name] = source
    shared_ids = [identities(page) for page in loaded[S].pages]
    separate_ids = [identities(page) for page in loaded[D].pages]
    assert shared_ids[0]["font"] == shared_ids[1]["font"]
    assert separate_ids[0]["font"] != separate_ids[1]["font"]
    assert shared_ids[0]["font"] == separate_ids[0]["font"], "same-number collision control disappeared"
    return loaded


def compare(candidate, mapping, loaded):
    assert len(candidate.pages) == len(mapping), "requested page count changed"
    expected_ids, actual_ids = [], []
    actual_pages = []
    for position, (name, old_index) in enumerate(mapping):
        source_page, output_page = loaded[name].pages[old_index], candidate.pages[position]
        # Resources first ensures a missing font is reported as an inventory
        # failure rather than accepted because extraction happened to recover.
        assert canonical(output_page.raw_get("/Resources")) == canonical(source_page.raw_get("/Resources")), f"page {position}: font/glyph/resource closure changed"
        assert inventory(output_page) == inventory(source_page), f"page {position}: content/text/geometry changed"
        expected_ids.append({kind: (name, identity) for kind, identity in identities(source_page).items()})
        actual_ids.append(identities(output_page))
        actual_pages.append((output_page.indirect_reference.idnum, output_page.indirect_reference.generation))
    assert len(set(actual_pages)) == len(mapping), "duplicated page occurrences reused a page object"
    for left in range(len(mapping)):
        for right in range(left):
            for kind in ("font", "glyph", "resources", "content"):
                assert (expected_ids[left][kind] == expected_ids[right][kind]) == (actual_ids[left][kind] == actual_ids[right][kind]), f"{kind}: sharing or source isolation changed between pages {left}/{right}"
    return {"page_mapping": mapping, "page_count": len(mapping), "observed_asset_ids": actual_ids, "checks": ["decoded_content", "type3_glyph_bytes", "resource_dictionaries", "text", "effective_boxes_and_rotation", "page_occurrence_identity", "shared_asset_identity", "distinct_input_isolation"]}


def corrupted_control(path, mutation):
    writer = pypdf.PdfWriter(clone_from=reader(path))
    resources = writer.pages[0]["/Resources"]
    if mutation == "missing-font":
        del resources["/Font"]["/F1"]
    elif mutation == "altered-glyph":
        glyph = resources["/Font"]["/F1"]["/CharProcs"]["/A"]
        glyph.set_data(glyph.get_data() + b"\n0 0 1 1 re f\n")
    else:
        raise AssertionError(mutation)
    output = EVIDENCE / f"import-control-{mutation}.pdf"
    with output.open("wb") as file:
        writer.write(file)
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["source", "verify"])
    args = parser.parse_args()
    tool = checker()
    loaded = sources()
    EVIDENCE.mkdir(exist_ok=True)
    result = {"checker": tool, "sources": SOURCE_HASHES, "scope": "two original generated Type3 fixtures; independent structural/content checks only", "source_manifest_sha256": sha((FIXTURES / "manifest.json").read_bytes())}
    if args.mode == "source":
        assert not WARNINGS, WARNINGS
        result["result"] = "source_checks_passed"
        (EVIDENCE / "import-sources.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print("import source checks passed")
        return
    cases = {}
    for name, mapping in CASES.items():
        path = EVIDENCE / f"{name}.pdf"
        cases[name] = compare(reader(path), mapping, loaded) | {"candidate_sha256": sha(path.read_bytes()), "result": "passed"}
    controls = {}
    for mutation in ("missing-font", "altered-glyph"):
        path = corrupted_control(EVIDENCE / "import-shared-duplicate.pdf", mutation)
        candidate = reader(path)
        assert len(candidate.pages) == 3, "control must remain independently reparseable"
        try:
            compare(candidate, CASES["import-shared-duplicate"], loaded)
        except AssertionError as error:
            controls[mutation] = {"candidate_sha256": sha(path.read_bytes()), "independent_reparse": "passed", "preservation": "failed_as_required", "reason": str(error)}
        else:
            raise AssertionError(f"corruption control {mutation} escaped detection")
    assert not WARNINGS, WARNINGS
    result |= {"result": "four_candidates_passed_and_two_corruptions_detected", "cases": cases, "controls": controls}
    (EVIDENCE / "import-results.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(result["result"])


if __name__ == "__main__":
    main()
