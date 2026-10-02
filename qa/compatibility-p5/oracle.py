# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Binas contributors.
"""Small producer-held-out corpus. No Binas writer is used by this oracle."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import io
import json
import logging
from pathlib import Path
import re
import shutil
import subprocess
import sys
import warnings

import pypdf
import reportlab
from pypdf.generic import (ArrayObject, DecodedStreamObject, DictionaryObject,
                           NameObject, NumberObject, TextStringObject)
from reportlab.pdfgen import canvas
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from PIL import Image, ImageChops

warnings.simplefilter("error")

if not __debug__:
    raise RuntimeError("Required assertions: run without -O")
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("p3_helpers", HERE.parent / "retained-ascii" / "oracle.py")
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
OLD, NEW = "ALPHA", "OMEGA-LONG"
REGION = [35, 143, 180, 170]  # PDF coordinates, fixed before Binas execution.


def sha(data):
    return hashlib.sha256(data).hexdigest()


def dictionary(**values):
    return DictionaryObject({NameObject("/" + key): value for key, value in values.items()})


def stream(data):
    value = DecodedStreamObject()
    value.set_data(data)
    return value


def pypdf_fixture(path, golden=False, raw=False, shared=False, missing=False):
    writer = pypdf.PdfWriter()
    writer.add_metadata({"/Title": "Original Apache-2.0 P5 pypdf fixture", "/Producer": "pypdf " + pypdf.__version__})
    pages = [writer.add_blank_page(width=240, height=200) for _ in range(2)]
    font = None
    if not raw:
        codes = [code for code in range(32, 91) if not (missing and code == 45)]
        cmap = ("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n"
                "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n"
                "/CMapName /P5ASCII def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n"
                + str(len(codes)) + " beginbfchar\n"
                + "\n".join(f"<{code:02X}> <{code:04X}>" for code in codes)
                + "\nendbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n")
        cmap_ref = writer._add_object(stream(cmap.encode()))
        font = writer._add_object(dictionary(Type=NameObject("/Font"), Subtype=NameObject("/Type1"),
                BaseFont=NameObject("/Helvetica"), Encoding=NameObject("/WinAnsiEncoding"), ToUnicode=cmap_ref))
    target = NEW if golden else OLD
    selected = b"" if raw else b"/F1 14 Tf "
    payload = b"BT " + selected + b"1 0 0 1 40 150 Tm (" + target.encode() + b") Tj 0 -30 Td (KEEP) Tj ET\n"
    first_ref = writer._add_object(stream(payload))
    for index, page in enumerate(pages):
        page[NameObject("/Resources")] = dictionary() if raw else dictionary(Font=dictionary(F1=font))
        page[NameObject("/Contents")] = first_ref if index == 0 or shared else writer._add_object(stream(
            b"BT " + selected + b"1 0 0 1 40 150 Tm (UNTOUCHED PAGE) Tj ET\n"))
    writer.add_attachment("original.txt", b"ORIGINAL LICENSED ATTACHMENT\n")
    writer._add_object(stream(b"UNREACHABLE P5 SENTINEL\n"))
    with path.open("wb") as output:
        writer.write(output)


def reportlab_fixture(path, golden=False, standard=False):
    font = "Helvetica" if standard else "P5Vera"
    output = canvas.Canvas(str(path), pagesize=(240, 200), invariant=1, pageCompression=1)
    output.setTitle("Original Apache-2.0 P5 ReportLab fixture")
    for page in range(2):
        output.setFont(font, 14)
        output.drawString(40, 150, (NEW if golden else OLD) if page == 0 else "UNTOUCHED PAGE")
        output.drawString(40, 120, "KEEP")
        # Keep all replacement glyphs in the original subset and golden subset.
        output.setFont(font, 6)
        output.drawString(10, 20, "ABCDEFGHIJKLMNOPQRSTUVWXYZ -")
        output.showPage()
    output.save()


def generate(directory):
    directory.mkdir(parents=True, exist_ok=True)
    pdf = directory / "pdf"
    pdf.mkdir(exist_ok=True)
    font_dir = Path(reportlab.__file__).parent / "fonts"
    pdfmetrics.registerFont(TTFont("P5Vera", str(font_dir / "Vera.ttf")))
    shutil.copyfile(font_dir / "bitstream-vera-license.txt", directory / "bitstream-vera-license.txt")
    entries = []
    cases = [
        ("pypdf-fonted", "fonted", {}, pypdf_fixture),
        ("pypdf-raw", "raw", {"raw": True}, pypdf_fixture),
        ("pypdf-shared", "refusal_shared", {"shared": True}, pypdf_fixture),
        ("pypdf-missing-glyph", "refusal_glyph", {"missing": True}, pypdf_fixture),
        ("reportlab-truetype", "fonted", {}, reportlab_fixture),
        ("reportlab-standard", "refusal_cmap", {"standard": True}, reportlab_fixture),
    ]
    for name, kind, options, producer in cases:
        path = pdf / f"{name}.pdf"
        producer(path, **options)
        reader = pypdf.PdfReader(path, strict=True)
        assert len(reader.pages) == 2
        contents = reader.pages[0].raw_get("/Contents")
        assert not isinstance(contents, ArrayObject)
        target = [contents.idnum, contents.generation]
        entry = {"id": name, "path": path.name, "sha256": sha(path.read_bytes()),
                 "source": f"Original local fixture generated by {producer.__name__}; no Binas writer or private document",
                 "license": "Apache-2.0" + (" AND Bitstream-Vera (notice ../bitstream-vera-license.txt)" if name == "reportlab-truetype" else ""),
                 "roles": ["compatibility", kind], "parse": "opens", "expected_pages": 2,
                 "kind": kind, "target": target, "producer": "ReportLab" if name.startswith("reportlab") else "pypdf",
                 "expected_edit": "success" if kind in ("raw", "fonted") else "reject",
                 "expected_error_code_edit": "unsafe_rewrite" if kind == "refusal_shared" else "unsupported_feature"}
        if kind in ("raw", "fonted"):
            golden = pdf / f"{name}-golden.pdf"
            producer(golden, golden=True, **options)
            entry.update(golden=golden.name, golden_sha256=sha(golden.read_bytes()))
        entries.append(entry)
    bad = pdf / "malformed-header.pdf"
    bad.write_bytes(b"This original Apache-2.0 negative is not a PDF.\n")
    entries.append({"id": "malformed-header", "path": bad.name, "sha256": sha(bad.read_bytes()),
                    "source": "Original local malformed-header control", "license": "Apache-2.0",
                    "roles": ["adversarial", "header"], "parse": "rejects", "expected_error_code": "invalid_syntax", "kind": "parse_reject"})
    manifest = {"version": 1, "corpus_root": "pdf", "fixture_count": len(entries),
                "coverage_status": "licensed_compatibility_and_adversarial", "fixtures": entries,
                "contract": {"old": OLD, "new": NEW, "page": 0, "match_index": 0, "shared": "Reject",
                             "target_change_region_pdf": REGION, "minimum_changed_rgb_pixels": 50},
                "environment": {"python": sys.version, "executable": sys.executable,
                                "pypdf": pypdf.__version__, "reportlab": reportlab.Version,
                                "vera_sha256": sha((font_dir / "Vera.ttf").read_bytes()),
                                "vera_license_sha256": sha((directory / "bitstream-vera-license.txt").read_bytes())}}
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"manifest": str(directory / "manifest.json"), "fixtures": len(entries)}))


def read(path, incremental=False):
    capture = helper.LogCapture()
    logger = logging.getLogger("pypdf")
    logger.addHandler(capture)
    try:
        reader = pypdf.PdfReader(path, strict=True)
        inventory, _ = helper.inventory(reader, path.read_bytes())
    finally:
        logger.removeHandler(capture)
    allowed = [helper.INCREMENTAL_READER_DIAGNOSTIC] if incremental else []
    assert capture.messages in ([], allowed), capture.messages
    return reader, inventory, capture.messages


def ops(reader, target):
    ref = pypdf.generic.IndirectObject(*target, reader)
    content = pypdf.generic.ContentStream(reader.get_object(ref), reader)
    return [{"operator": operator.decode(), "operands": [helper.canonical(value) for value in operands]}
            for operands, operator in content.operations]


def checks(source, output, golden, target, incremental=False):
    sr, si, sw = read(source)
    cr, ci, cw = read(output, incremental)
    gr, gi, gw = read(golden)
    target_key = f"{target[0]}:{target[1]}"
    target_dictionary = lambda inv: {key: value for key, value in inv[target_key]["value"]["dict"].items() if key != "/Length"}
    results = {"object_ids": set(si) == set(ci), "geometry": helper.geometry(sr) == helper.geometry(cr),
               "trailer": helper.canonical(sr.trailer) == helper.canonical(cr.trailer) if not incremental else
                 {k: helper.canonical(v) for k, v in sr.trailer.items() if k not in ('/Prev', '/XRefStm')} ==
                 {k: helper.canonical(v) for k, v in cr.trailer.items() if k not in ('/Prev', '/XRefStm')},
               "untouched_inventory": all(si[key]["value"] == ci[key]["value"] and
                  si[key].get("stream") == ci[key].get("stream") for key in si if key != target_key),
               "target_operations": ops(cr, target) == ops(gr, target),
               "target_dictionary": target_dictionary(si) == target_dictionary(ci),
               "target_decoded": cr.get_object(pypdf.generic.IndirectObject(*target, cr)).get_data() ==
                                  gr.get_object(pypdf.generic.IndirectObject(*target, gr)).get_data(),
               "text": [page.extract_text() for page in cr.pages] == [page.extract_text() for page in gr.pages],
               "revision_layout": (len(re.findall(rb"startxref\s+\d+\s+%%EOF", output.read_bytes())) == (2 if incremental else 1)
                    and (int(cr.trailer["/Prev"]) == helper.startxref(source.read_bytes()) if incremental else "/Prev" not in cr.trailer)),
               "prefix": not incremental or output.read_bytes().startswith(source.read_bytes())}
    return results, {"source_warnings": sw, "output_warnings": cw, "golden_warnings": gw,
                     "source_inventory": si, "output_inventory": ci, "golden_inventory": gi}


def parse_info(path):
    command = [str(helper.POPPLER / "pdfinfo.exe"), "-box", "-f", "1", "-l", "2", str(path)]
    run = subprocess.run(command, capture_output=True, text=True, check=False)
    result = {"command": command, "exit_code": run.returncode, "stdout": run.stdout, "stderr": run.stderr}
    assert run.returncode == 0 and not run.stderr, result
    result["geometry"] = [line.strip() for line in run.stdout.splitlines()
       if re.match(r"^(Pages:|Page\s+\d+\s+(size:|rot:|MediaBox:|CropBox:|BleedBox:|TrimBox:|ArtBox:))", line)]
    assert any(re.match(r"^Pages:\s+2$", line) for line in result["geometry"])
    return result


def independent_append(source, golden, target, destination):
    """pypdf serializes the golden stream; a tiny independent append pins warning policy."""
    sr, gr = pypdf.PdfReader(source, strict=True), pypdf.PdfReader(golden, strict=True)
    encoded = io.BytesIO()
    gr.get_object(pypdf.generic.IndirectObject(*target, gr)).write_to_stream(encoded)
    data = bytearray(source.read_bytes() + b"\n")
    offset = len(data)
    data += f"{target[0]} {target[1]} obj\n".encode() + encoded.getvalue() + b"\nendobj\n"
    xref = len(data)
    data += f"xref\n{target[0]} 1\n{offset:010d} {target[1]:05d} n \ntrailer\n".encode()
    trailer = DictionaryObject({key: value for key, value in sr.trailer.items() if key not in ("/Prev", "/XRefStm")})
    trailer[NameObject("/Prev")] = NumberObject(helper.startxref(source.read_bytes()))
    serialized = io.BytesIO()
    trailer.write_to_stream(serialized)
    data += serialized.getvalue() + f"\nstartxref\n{xref}\n%%EOF\n".encode()
    destination.write_bytes(data)
    result, detail = checks(source, destination, golden, target, incremental=True)
    assert all(result.values()), result
    assert detail["output_warnings"] == [helper.INCREMENTAL_READER_DIAGNOSTIC], detail["output_warnings"]
    return {"checks": result, "pypdf_warnings": detail["output_warnings"], "sha256": sha(data)}


def verify(directory, outputs, reports):
    reports.mkdir(parents=True, exist_ok=True)
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    helper.renderer_baseline(reports)
    all_reports = []
    intake = []
    for case in manifest["fixtures"]:
        source = directory / "pdf" / case["path"]
        assert sha(source.read_bytes()) == case["sha256"]
        if case["parse"] == "opens":
            reader, _, diagnostic = read(source)
            assert OLD in reader.pages[0].extract_text()
            intake.append({"id": case["id"], "pypdf_warnings": diagnostic, "pdfinfo": parse_info(source),
                           "text": [page.extract_text() for page in reader.pages]})
        else:
            try:
                pypdf.PdfReader(source, strict=True)
            except pypdf.errors.PdfReadError as error:
                intake.append({"id": case["id"], "correct_parse_rejection": type(error).__name__, "message": str(error)})
            else:
                raise AssertionError("Independent parser accepted malformed header")
    for case in manifest["fixtures"]:
        if case.get("expected_edit") != "success":
            continue
        source, golden = directory / "pdf" / case["path"], directory / "pdf" / case["golden"]
        assert sha(golden.read_bytes()) == case["golden_sha256"]
        control = independent_append(source, golden, case["target"], reports / f"{case['id']}-independent-incremental.pdf")
        for mode in ("compact", "incremental"):
            output = outputs / f"{case['id']}-{mode}.pdf"
            result, detail = checks(source, output, golden, case["target"], mode == "incremental")
            report = {"id": case["id"], "mode": mode, "checks": result, "detail": detail,
                      "independent_incremental_control": control,
                      "source_sha256": sha(source.read_bytes()), "output_sha256": sha(output.read_bytes())}
            infos = [parse_info(path) for path in (source, output, golden)]
            result["poppler_parse_geometry"] = infos[0]["geometry"] == infos[1]["geometry"] == infos[2]["geometry"]
            report["pdfinfo"] = infos
            if case["kind"] == "fonted":
                expected_stderr = "" if case["producer"] == "ReportLab" else helper.AMBIENT_RENDER_DIAGNOSTICS
                renders = [helper.render(path, reports / f"{case['id']}-{mode}-{role}", pages=(1, 2), expected_stderr=expected_stderr)
                           for role, path in (("source", source), ("output", output), ("golden", golden))]
                report["renders"] = renders
                for page in (0, 1):
                    actual = renders[1]["pages"][page]
                    expected = renders[2]["pages"][page]
                    result[f"page_{page + 1}_golden_pixels"] = actual["dimensions"] == expected["dimensions"] and actual["pixels_sha256"] == expected["pixels_sha256"]
                result["untouched_page_pixels"] = renders[0]["pages"][1]["pixels_sha256"] == renders[1]["pages"][1]["pixels_sha256"]
                with Image.open(renders[0]["pages"][0]["file"]) as before, Image.open(renders[2]["pages"][0]["file"]) as after:
                    diff = ImageChops.difference(before.convert("RGB"), after.convert("RGB"))
                    bounds = diff.getbbox()
                    changed = sum(pixel != (0, 0, 0) for pixel in diff.get_flattened_data())
                    # 96 DPI, fixed page height 200 PDF units; outer rounding.
                    region = [46, 40, 240, 76]
                    result["visible_target_effect"] = changed > 50 and bounds is not None and bounds[0] >= region[0] and bounds[1] >= region[1] and bounds[2] <= region[2] and bounds[3] <= region[3]
                    report["target_diff"] = {"changed_pixels": changed, "bounds": bounds, "allowed_region_pixels": region}
            else:
                report["appearance"] = "NotChecked: raw fontless profile"
            (reports / f"{case['id']}-{mode}.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
            assert all(result.values()), {"id": case["id"], "mode": mode, "checks": result}
            all_reports.append({"id": case["id"], "mode": mode, "checks": result})
    # Intentional wrong source and collateral damage: comparator itself must fail.
    fonted = next(case for case in manifest["fixtures"] if case["id"] == "pypdf-fonted")
    source = directory / "pdf" / fonted["path"]
    golden = directory / "pdf" / fonted["golden"]
    wrong, _ = checks(source, source, golden, fonted["target"])
    assert not wrong["target_operations"] and not wrong["target_decoded"] and not wrong["text"]
    candidate = outputs / "pypdf-fonted-compact.pdf"
    mutant = reports / "collateral-mutant.pdf"
    data = candidate.read_bytes()
    assert b"UNREACHABLE P5 SENTINEL" in data
    mutant.write_bytes(data.replace(b"UNREACHABLE P5 SENTINEL", b"COLLATERAL! P5 SENTINEL"))
    damaged, _ = checks(source, mutant, golden, fonted["target"])
    assert not damaged["untouched_inventory"]
    # The same exact target-pixel comparator must reject a missing target edit.
    wrong_render = helper.render(source, reports / "missing-target-render", pages=(1, 2))
    good_render = helper.render(golden, reports / "expected-target-render", pages=(1, 2))
    assert wrong_render["pages"][0]["pixels_sha256"] != good_render["pages"][0]["pixels_sha256"]
    summary = {"status": "Passed", "intake": intake, "cases": all_reports, "negative_controls": {
                "missing_target": {"failed_checks": [key for key, value in wrong.items() if not value]},
                "collateral_sentinel": {"failed_checks": [key for key, value in damaged.items() if not value]},
                "missing_target_render": {"target_pixel_comparison_failed": True}},
               "limits": ["Two installed producers; synthetic original documents", "No forms/tags claim",
                          "Raw appearance NotChecked", "Qualified Poppler ambient font diagnostics retained"]}
    (reports / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": summary["status"], "intakes": len(intake), "exports": len(all_reports),
                      "negative_controls": summary["negative_controls"], "summary": str(reports / "summary.json")}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=["generate", "verify"])
    parser.add_argument("directory", type=Path)
    parser.add_argument("outputs", nargs="?", type=Path)
    parser.add_argument("reports", nargs="?", type=Path)
    args = parser.parse_args()
    if args.action == "generate":
        generate(args.directory)
    else:
        verify(args.directory, args.outputs, args.reports)
