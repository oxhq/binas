# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Binas P3b local QA contributors.
"""Independent, fixed-fixture oracle for public fonted page_text replacement."""
from __future__ import annotations

import argparse
import importlib.util
import io
import json
import logging
import math
from pathlib import Path
import re
import sys
import warnings

import pypdf
import PIL
from pypdf.generic import IndirectObject
from PIL import Image, ImageChops

if not __debug__:
    raise RuntimeError("Run without -O: required checks must remain active")

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("retained_ascii_helpers", HERE.parent / "retained-ascii" / "oracle.py")
helper = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(helper)
TARGET = b"BT /F1 14 Tf 1 0 0 1 40 150 Tm (ALPHA) Tj 0 -30 Td (KEEP) Tj ET\n"
EXPECTED = TARGET.replace(b"(ALPHA)", b"(OMEGA-LONG)")
CHANGE_REGION = [35, 143, 170, 169]
FIXTURES = HERE.parent.parent / "crates" / "binas-pdf" / "tests" / "fixtures" / "fonted-text"


def cmap(mapping=None):
    mapping = mapping if mapping is not None else {code: code for code in range(32, 91)}
    lines = [b"/CIDInit /ProcSet findresource begin", b"12 dict begin", b"begincmap", b"/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def", b"/CMapName /P3bExplicitASCII def", b"/CMapType 2 def", b"1 begincodespacerange", b"<00> <FF>", b"endcodespacerange", f"{len(mapping)} beginbfchar".encode()]
    lines += [f"<{code:02X}> <{value:04X}>".encode() for code, value in sorted(mapping.items())]
    lines += [b"endbfchar", b"endcmap", b"CMapName currentdict /CMap defineresource pop", b"end", b"end", b""]
    return b"\n".join(lines)


def fixture_objects(flate=False, expected=False):
    objects = helper.fixture_objects(flate)
    objects[3] = objects[3].replace(b"/Resources << >>", b"/Resources 7 0 R")
    objects[4] = helper.stream(EXPECTED if expected else TARGET, flate=flate)
    objects[8] = objects[8].replace(b"/Encoding /WinAnsiEncoding", b"/Encoding /WinAnsiEncoding /ToUnicode 27 0 R")
    objects[27] = helper.stream(cmap())
    return objects


def generate(directory):
    directory.mkdir(parents=True, exist_ok=True)
    manifest = {"license": "Apache-2.0", "provenance": "Original local generator; fixed helper sources reused from frozen P3a; no downloaded corpus", "target_object": [4, 0], "shared_font_object": [8, 0], "shared_resources_object": [7, 0], "ToUnicode_object": [27, 0], "sentinel_object": [28, 0], "replacement": {"page": 0, "match_ordinal": 0, "old": "ALPHA", "new": "OMEGA-LONG", "shared_policy": "Reject"}, "predeclared_target_change_region_pdf": CHANGE_REGION, "fixtures": []}
    variants = {}
    for name, flate in [("plain", False), ("flate", True)]:
        variants[f"fonted-text-{name}.pdf"] = fixture_objects(flate)
        variants[f"fonted-text-{name}-golden.pdf"] = fixture_objects(flate, expected=True)
    no_cmap = fixture_objects()
    no_cmap[8] = no_cmap[8].replace(b" /ToUnicode 27 0 R", b"")
    variants["fonted-text-missing-cmap.pdf"] = no_cmap
    missing = fixture_objects()
    missing[27] = helper.stream(cmap({code: code for code in range(32, 91) if code != 45}))
    variants["fonted-text-missing-replacement.pdf"] = missing
    ambiguous = fixture_objects()
    ambiguous[27] = helper.stream(cmap({code: (65 if code == 66 else code) for code in range(32, 91)}))
    variants["fonted-text-ambiguous-cmap.pdf"] = ambiguous
    shared = fixture_objects()
    shared[5] = shared[5].replace(b"/Contents 6 0 R", b"/Contents 4 0 R")
    variants["fonted-text-shared-contents.pdf"] = shared
    for name, objects in variants.items():
        data = helper.write_pdf(objects)
        path = directory / name
        path.write_bytes(data)
        reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
        assert len(reader.pages) == 3
        if name in ["fonted-text-plain.pdf", "fonted-text-flate.pdf"]:
            assert reader.pages[0].extract_text().strip() == "ALPHA\nKEEP"
        if "golden" in name:
            assert reader.pages[0].extract_text().strip() == "OMEGA-LONG\nKEEP"
        manifest["fixtures"].append({"name": name, "sha256": helper.sha(data), "bytes": len(data), "objects": len(objects), "generation": 0, "trailer_size": 29, "role": "independently generated expected document" if "golden" in name else "public source fixture or refusal control"})
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


def parse_control(source, directory):
    original = pypdf.PdfReader(io.BytesIO(source), strict=True)
    flate = original.get_object(IndirectObject(4, 0, original)).get("/Filter") == "/FlateDecode"
    data = helper.write_pdf({4: helper.stream(EXPECTED, flate=flate)}, source)
    path = directory / "independent-incremental-control.pdf"
    path.write_bytes(data)
    local = helper.LogCapture()
    logger = logging.getLogger("pypdf")
    logger.addHandler(local)
    try:
        reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
        entries, _ = helper.inventory(reader, data)
        if set(entries) != {f"{i}:0" for i in range(1, 29)} or helper.operations(reader) != helper.operations(pypdf.PdfReader(io.BytesIO(helper.write_pdf(fixture_objects(flate, True))), strict=True)):
            raise RuntimeError("independent Incremental control identities/operations mismatch")
        offsets = []
        for generation, records in reader.xref.items():
            for number, offset in records.items():
                if number:
                    if not re.match(fr"{number}\s+{generation}\s+obj\b".encode(), data[offset:]):
                        raise RuntimeError("independent Incremental control xref id/offset mismatch")
                    offsets.append({"id": [number, generation], "offset": offset})
        if local.messages != [helper.INCREMENTAL_READER_DIAGNOSTIC]:
            raise RuntimeError(f"independent Incremental control diagnostics mismatch: {local.messages}")
        result = {"status": "Passed", "file": str(path), "sha256": helper.sha(data), "reader_diagnostics": local.messages, "xref_offsets_verified": sorted(offsets, key=lambda item: item["id"]), "strict": True, "target_operations": helper.operations(reader)}
        (directory / "independent-incremental-control.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        return result
    finally:
        logger.removeHandler(local)


def profile(reader):
    pages = reader.pages
    resources0 = pages[0].raw_get("/Resources")
    resources1 = pages[1].raw_get("/Resources")
    resource = resources0.get_object()
    font_ref = resource["/Font"].raw_get("/F1")
    font = font_ref.get_object()
    cmap_ref = font.raw_get("/ToUnicode") if "/ToUnicode" in font else None
    cmap_bytes = cmap_ref.get_object().get_data() if cmap_ref is not None else None
    return {"resource_page0": helper.canonical(resources0), "resource_page1": helper.canonical(resources1), "selected_font_ref": helper.canonical(font_ref), "font": helper.canonical(font), "ToUnicode_ref": helper.canonical(cmap_ref) if cmap_ref is not None else None, "ToUnicode_decoded_sha256": helper.sha(cmap_bytes) if cmap_bytes is not None else None, "ToUnicode_exact_ascii_map": cmap_bytes == cmap()}


def target_difference(before, golden):
    with Image.open(before) as original_image, Image.open(golden) as golden_image:
        original_image, golden_image = original_image.convert("RGB"), golden_image.convert("RGB")
        if original_image.size != golden_image.size:
            raise RuntimeError("source/golden target dimensions differ")
        diff = ImageChops.difference(original_image, golden_image)
        bbox = diff.getbbox()
        pixels = sum(1 for pixel in diff.get_flattened_data() if any(pixel))
        # Fixed media/crop box [0,0,240,210], no rotation, 96 DPI.
        x0, y0, x1, y1 = CHANGE_REGION
        region = [math.floor(x0 * 96 / 72), math.floor((210 - y1) * 96 / 72), math.ceil(x1 * 96 / 72), math.ceil((210 - y0) * 96 / 72)]
        confined = bbox is not None and bbox[0] >= region[0] and bbox[1] >= region[1] and bbox[2] <= region[2] and bbox[3] <= region[3]
        return {"declared_region_pdf": CHANGE_REGION, "declared_region_pixels": region, "actual_difference_bbox": list(bbox) if bbox else None, "changed_pixels": pixels, "material_pixel_threshold_exclusive": 50, "confined": confined}


def verify(source, output, mode, report_path, render_only=False):
    report = {"status": "Failed", "scope": "P3b fixed standard Type1 Helvetica WinAnsi explicit injective ToUnicode public page_text replacement", "oracle_runtime": {"python_executable": sys.executable, "python_version": sys.version, "pypdf_version": pypdf.__version__, "pypdf_module": pypdf.__file__, "Pillow_version": PIL.__version__, "oracle_source_sha256": helper.sha(Path(__file__).read_bytes()), "helper_source_sha256": helper.sha(Path(helper.__file__).read_bytes())}, "source": str(source), "output": str(output), "mode": mode, "checks": [], "failures": [], "predeclared_target_change_region_pdf": CHANGE_REGION}
    report_path.parent.mkdir(parents=True, exist_ok=True)
    capture = helper.LogCapture()
    logger = logging.getLogger("pypdf")
    logger.addHandler(capture)
    def check(name, passed, detail=None):
        report["checks"].append({"name": name, "status": "Passed" if passed else "Failed", "detail": detail})
        if not passed:
            report["failures"].append(name)
    try:
        if pypdf.__version__ != "6.10.0":
            raise RuntimeError("required installed pypdf 6.10.0 unavailable")
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            a, b = source.read_bytes(), output.read_bytes()
            report["source_sha256"], report["output_sha256"] = helper.sha(a), helper.sha(b)
            original = pypdf.PdfReader(io.BytesIO(a), strict=True)
            source_warnings = list(capture.messages)
            start = len(capture.messages)
            candidate = pypdf.PdfReader(io.BytesIO(b), strict=True)
            candidate_warnings = capture.messages[start:]
            flate = a == helper.write_pdf(fixture_objects(True))
            golden_bytes = helper.write_pdf(fixture_objects(flate, expected=True))
            golden = pypdf.PdfReader(io.BytesIO(golden_bytes), strict=True)
            report["golden_sha256"] = helper.sha(golden_bytes)
            control_warnings = []
            if mode == "Incremental":
                parse_dir = report_path.parent / (report_path.stem + "-parse")
                parse_dir.mkdir(parents=True, exist_ok=True)
                start = len(capture.messages)
                report["incremental_reader_control"] = parse_control(a, parse_dir)
                control_warnings = capture.messages[start:]
            report["parser_phase_warnings"] = {"source": source_warnings, "candidate_latest_reader": candidate_warnings, "independent_incremental_control": control_warnings}
            before, _ = helper.inventory(original, a)
            after, _ = helper.inventory(candidate, b)
            report["inventory_before"], report["inventory_after"] = before, after
            report["geometry_before"], report["geometry_after"] = helper.geometry(original), helper.geometry(candidate)
            report["operations_before"], report["operations_after"], report["operations_expected"] = helper.operations(original), helper.operations(candidate), helper.operations(golden)
            report["profile_before"], report["profile_after"] = profile(original), profile(candidate)
            if not render_only:
                check("exact trusted deterministic source fixture bytes", a in [helper.write_pdf(fixture_objects(False)), helper.write_pdf(fixture_objects(True))])
                check("source fixture object identities", set(before) == {f"{i}:0" for i in range(1, 29)})
                check("retained object identities and generations", set(before) == set(after))
                changed = []
                for key in before.keys() & after.keys():
                    left, right = before[key], after[key]
                    if key == "4:0":
                        ldict, rdict = dict(left["value"]["dict"]), dict(right["value"]["dict"])
                        ldict.pop("/Length"); rdict.pop("/Length")
                        check("target dictionary except Length", ldict == rdict)
                    elif left["value"] != right["value"] or left.get("stream") != right.get("stream"):
                        changed.append(key)
                check("all non-target dictionaries and stream payloads", not changed, sorted(changed))
                check("page geometry and page tree", report["geometry_before"] == report["geometry_after"] and len(candidate.pages) == 3)
                check("source target exact decoded bytes", original.get_object(IndirectObject(4, 0, original)).get_data() == TARGET)
                check("independent ContentStream exact operators and operands", report["operations_after"] == report["operations_expected"])
                report["target_decoded_bytes_before"] = original.get_object(IndirectObject(4, 0, original)).get_data().hex()
                report["target_decoded_bytes_after"] = candidate.get_object(IndirectObject(4, 0, candidate)).get_data().hex()
                report["target_decoded_bytes_expected"] = EXPECTED.hex()
                report["target_decoded_bytes_equal_golden"] = candidate.get_object(IndirectObject(4, 0, candidate)).get_data() == EXPECTED
                check("exact replacement target decoded bytes", report["target_decoded_bytes_equal_golden"])
                check("exact selected Helvetica WinAnsi ToUnicode profile", report["profile_after"]["font"] == report["profile_before"]["font"] and report["profile_after"]["selected_font_ref"] == {"ref": [8, 0]})
                check("target and untouched page shared font/resource identities", report["profile_after"]["resource_page0"] == report["profile_after"]["resource_page1"] == {"ref": [7, 0]} and report["profile_after"]["selected_font_ref"] == {"ref": [8, 0]})
                check("exact explicit injective ToUnicode CMap", report["profile_after"]["ToUnicode_ref"] == {"ref": [27, 0]} and report["profile_after"]["ToUnicode_exact_ascii_map"])
                check("independent target Unicode extraction", original.pages[0].extract_text().strip() == "ALPHA\nKEEP" and candidate.pages[0].extract_text().strip() == "OMEGA-LONG\nKEEP")
                check("unaffected fonted extracted text", [page.extract_text() for page in original.pages[1:]] == [page.extract_text() for page in candidate.pages[1:]])
                report["trailer_before"], report["trailer_after"] = helper.canonical(original.trailer), helper.canonical(candidate.trailer)
                ta, tb = dict(report["trailer_before"]["dict"]), dict(report["trailer_after"]["dict"])
                pa, pb = ta.pop("/Prev", None), tb.pop("/Prev", None)
                check("all retained trailer values including ID Info Root Size", ta == tb and ta["/Size"] == {"number": 29})
                expected_prev = {"number": helper.startxref(a)} if mode == "Incremental" else None
                check("exact prescribed Prev", pa is None and pb == expected_prev)
                revisions = len(re.findall(rb"startxref\s+\d+\s+%%EOF", b))
                check("exact prescribed revision count", revisions == (2 if mode == "Incremental" else 1), revisions)
                if mode == "Incremental":
                    check("exact original byte prefix", b.startswith(a))
            render_dir = report_path.parent / (report_path.stem + "-render")
            render_dir.mkdir(parents=True, exist_ok=True)
            golden_path = render_dir / "independent-golden.pdf"
            golden_path.write_bytes(golden_bytes)
            if len(original.pages) == len(candidate.pages) == 3:
                report["pdfinfo_before"], report["pdfinfo_after"], report["pdfinfo_golden"] = helper.pdfinfo(source), helper.pdfinfo(output), helper.pdfinfo(golden_path)
                check("independent Poppler parser warning-free page geometry", report["pdfinfo_before"]["page_geometry_lines"] == report["pdfinfo_after"]["page_geometry_lines"] == report["pdfinfo_golden"]["page_geometry_lines"])
                report["renderer_baseline"] = helper.renderer_baseline(render_dir)
                report["render_before"] = helper.render(source, render_dir / "source", pages=(1, 2, 3))
                report["render_after"] = helper.render(output, render_dir / "output", pages=(1, 2, 3))
                report["render_golden"] = helper.render(golden_path, render_dir / "golden", pages=(1, 2, 3))
                check("exact characterized renderer ambient diagnostics", report["render_before"]["stderr"] == report["render_after"]["stderr"] == report["render_golden"]["stderr"] == report["renderer_baseline"]["pinned_ambient_stderr"])
                left, right, expected_images = report["render_before"]["pages"], report["render_after"]["pages"], report["render_golden"]["pages"]
                def pixels(page):
                    return page["dimensions"], page["mode"], page["pixels_sha256"]
                check("target page exact independently generated golden pixels", pixels(right[0]) == pixels(expected_images[0]))
                check("unaffected valid page render dimensions and exact pixels", [pixels(page) for page in left[1:]] == [pixels(page) for page in right[1:]] == [pixels(page) for page in expected_images[1:]])
                report["target_source_to_golden_difference"] = target_difference(left[0]["file"], expected_images[0]["file"])
                check("material expected target change confined to declared region", report["target_source_to_golden_difference"]["changed_pixels"] > 50 and report["target_source_to_golden_difference"]["confined"])
            else:
                check("required three rendered pages present", False)
            report["parser_warnings"] = capture.messages + [str(item.message) for item in caught]
            expected_warnings = [helper.INCREMENTAL_READER_DIAGNOSTIC] if mode == "Incremental" else []
            check("source independent parser warning-free", not source_warnings)
            check("exact phase-qualified independent parser diagnostics", candidate_warnings == expected_warnings and control_warnings == expected_warnings and capture.messages == expected_warnings + expected_warnings and not caught)
            report["parser_qualification"] = "qualified diagnostic-bearing strict parsing" if mode == "Incremental" else "warning-free strict parsing"
            report["renderer_qualification"] = "diagnostic-bearing fixed-renderer target/golden and untouched-page pixel comparison; no general conformance/font assurance"
        report["status"] = "Passed" if not report["failures"] else "Failed"
    except Exception as exc:
        report["failures"].append("oracle exception")
        report["exception"] = f"{type(exc).__name__}: {exc}"
        report["parser_warnings"] = capture.messages
    finally:
        logger.removeHandler(capture)
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def self_test(source, output, mode, directory):
    directory.mkdir(parents=True, exist_ok=True)
    positive = verify(source, output, mode, directory / "positive.json")
    data = source.read_bytes()
    reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
    _, objects = helper.inventory(reader, data)
    flate = data == helper.write_pdf(fixture_objects(True))
    good = {**objects, 4: helper.stream(EXPECTED, flate=flate)}
    cases = {
        "wrong-font": ({8: objects[8].replace(b"/Helvetica", b"/Times-Roman")}, "exact selected Helvetica WinAnsi ToUnicode profile"),
        "target-missing": ({4: helper.stream(EXPECTED.replace(b"(OMEGA-LONG)", b"()"), flate=flate)}, "independent ContentStream exact operators and operands"),
        "wrong-target": ({4: helper.stream(EXPECTED.replace(b"OMEGA-LONG", b"SIGMA-LONG"), flate=flate)}, "independent ContentStream exact operators and operands"),
        "collateral-target-text": ({4: helper.stream(EXPECTED.replace(b"KEEP", b"LOST"), flate=flate)}, "independent ContentStream exact operators and operands"),
        "target-trailing-comment": ({4: helper.stream(EXPECTED + b"% collateral tail\n", flate=flate)}, "exact replacement target decoded bytes"),
        "out-of-scope-text": ({6: helper.stream(b"BT /F1 14 Tf 1 0 0 1 35 165 Tm (DAMAGED PAGE ONE) Tj ET\n")}, "all non-target dictionaries and stream payloads"),
        "shared-resource-font": ({7: objects[7].replace(b"/F1 8 0 R", b"/F1 13 0 R")}, "target and untouched page shared font/resource identities"),
        "CMap-damage": ({27: helper.stream(cmap({code: (81 if code == 79 else code) for code in range(32, 91)}))}, "exact explicit injective ToUnicode CMap"),
        "image-damage": ({9: helper.stream(bytes([0] * 12), b" /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", True)}, "all non-target dictionaries and stream payloads"),
        "unreferenced-sentinel-change": ({28: helper.stream(b"LOST SENTINEL", b" /QASentinel true", True)}, "all non-target dictionaries and stream payloads"),
    }
    results = []
    for name, (mutation, intended) in cases.items():
        modified = {**good, **mutation}
        changes = {number: body for number, body in modified.items() if body != objects[number]}
        mutant_bytes = helper.write_pdf(changes, data) if mode == "Incremental" else helper.write_pdf(modified)
        mutant = directory / f"{name}.pdf"
        mutant.write_bytes(mutant_bytes)
        result = verify(source, mutant, mode, directory / f"{name}.json")
        results.append({"name": name, "mutant": str(mutant), "sha256": helper.sha(mutant_bytes), "intended_failure": intended, "rejected": result["status"] == "Failed" and not result.get("exception") and intended in result["failures"], "failures": result["failures"], "exception": result.get("exception")})
    for name in ["wrong-font", "target-missing"]:
        result = verify(source, directory / f"{name}.pdf", mode, directory / f"render-only-{name}.json", render_only=True)
        intended = "target page exact independently generated golden pixels"
        results.append({"name": f"render-only-{name}", "intended_failure": intended, "rejected": result["status"] == "Failed" and not result.get("exception") and intended in result["failures"], "failures": result["failures"], "exception": result.get("exception")})
    summary = {"status": "Passed" if positive["status"] == "Passed" and all(item["rejected"] for item in results) else "Failed", "positive_status": positive["status"], "mutants": results, "target_appearance": "Compared to independently generated golden; material source/golden change confined to predeclared region", "scope": "bounded self-test; synthetic mutants are not Binas exports"}
    (directory / "self-test.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    gen = sub.add_parser("generate")
    gen.add_argument("directory", nargs="?", type=Path, default=FIXTURES)
    for name in ["verify", "self-test"]:
        cmd = sub.add_parser(name)
        cmd.add_argument("source", type=Path)
        cmd.add_argument("output", type=Path)
        cmd.add_argument("mode", choices=["Incremental", "Compact"])
        cmd.add_argument("report" if name == "verify" else "directory", type=Path)
    args = parser.parse_args()
    if args.command == "generate":
        result = generate(args.directory)
        print(json.dumps(result, indent=2))
        return 0
    result = verify(args.source, args.output, args.mode, args.report) if args.command == "verify" else self_test(args.source, args.output, args.mode, args.directory)
    print(json.dumps({key: result.get(key) for key in ["status", "failures", "exception", "positive_status", "mutants"] if key in result}, indent=2))
    return 0 if result["status"] == "Passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
