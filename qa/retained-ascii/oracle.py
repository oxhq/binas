# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Binas P3a local QA contributors.
"""Bounded, independent retained-ASCII fixture/oracle. No Binas imports."""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import logging
from pathlib import Path
import re
import subprocess
import sys
import warnings
import zlib

import pypdf
import PIL
from pypdf.generic import (
    ArrayObject, BooleanObject, ByteStringObject, ContentStream, DictionaryObject,
    FloatObject, IndirectObject, NameObject, NullObject, NumberObject,
    StreamObject, TextStringObject,
)
from PIL import Image

HERE = Path(__file__).resolve().parent
if not __debug__:
    raise RuntimeError("Run without -O: required checks must remain active")
POPPLER = Path(r"C:\Users\garae\.cache\codex-runtimes\codex-primary-runtime\dependencies\native\poppler\Library\bin")
TARGET_ID = 4
TARGET = b"BT 1 0 0 1 40 150 Tm (ALPHA) Tj 0 -30 Td (KEEP) Tj ET\n"
EXPECTED = TARGET.replace(b"(ALPHA)", b"(OMEGA)")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def stream(data, extra=b"", flate=False):
    if flate:
        data = zlib.compress(data, 6)
        extra += b" /Filter /FlateDecode"
    return b"<< /Length " + str(len(data)).encode() + extra + b" >>\nstream\n" + data + b"\nendstream"


def fixture_objects(flate=False):
    """Original deterministic sources, intentionally fontless target on page 0."""
    return {
        1: b"<< /Type /Catalog /Pages 2 0 R /Metadata 14 0 R /Outlines 16 0 R /Names 18 0 R /AcroForm 23 0 R >>",
        2: b"<< /Type /Pages /Kids [3 0 R 5 0 R 10 0 R] /Count 3 >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 240 210] /CropBox [0 0 240 210] /Rotate 0 /Resources << >> /Contents 4 0 R >>",
        4: stream(TARGET, flate=flate),
        5: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 220] /CropBox [10 15 280 205] /Rotate 0 /UserUnit 1 /Resources 7 0 R /Contents 6 0 R >>",
        6: stream(b"q 30 0 0 30 40 40 cm /Im1 Do Q\nBT /F1 14 Tf 1 0 0 1 35 165 Tm (UNTOUCHED PAGE ONE) Tj ET\n"),
        7: b"<< /Font << /F1 8 0 R >> /XObject << /Im1 9 0 R >> /ProcSet [/PDF /Text /ImageC] >>",
        8: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        9: stream(bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]), b" /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", True),
        10: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 360 240] /CropBox [15 10 345 225] /Rotate 90 /Resources 12 0 R /Contents 11 0 R /Annots [24 0 R] >>",
        11: stream(b"0.2 0.3 0.8 rg 25 35 180 45 re f\n0 0 0 rg BT /F2 16 Tf 1 0 0 1 30 175 Tm (UNTOUCHED PAGE TWO) Tj ET\n"),
        12: b"<< /Font << /F2 13 0 R >> /ProcSet [/PDF /Text] >>",
        13: b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>",
        14: stream(b'<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><qa>retained-ascii-original</qa></x:xmpmeta><?xpacket end="w"?>', b" /Type /Metadata /Subtype /XML"),
        15: b"<< /Title (Binas retained ASCII independent fixture) /Author (Local QA) /Subject (P3a preservation scope) /Creator (Original Apache-2.0 fixture generator) >>",
        16: b"<< /Type /Outlines /First 17 0 R /Last 17 0 R /Count 1 >>",
        17: b"<< /Title (Untouched page one) /Parent 16 0 R /Dest [5 0 R /Fit] >>",
        18: b"<< /Dests 19 0 R /EmbeddedFiles 20 0 R >>",
        19: b"<< /Names [(untouched-one) [5 0 R /XYZ 35 165 1]] >>",
        20: b"<< /Names [(qa-note.txt) 21 0 R] >>",
        21: b"<< /Type /Filespec /F (qa-note.txt) /UF (qa-note.txt) /Desc (Retained independent attachment) /EF << /F 22 0 R >> >>",
        22: stream(b"Original retained attachment bytes.\n", b" /Type /EmbeddedFile /Subtype /text#2Fplain"),
        23: b"<< /Fields [24 0 R] /NeedAppearances false /DR 26 0 R /DA (/F1 10 Tf 0 g) >>",
        24: b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (untouched-field) /V (FORM KEEP) /Rect [30 100 170 125] /P 10 0 R /F 4 /DA (/F1 10 Tf 0 g) /AP << /N 25 0 R >> >>",
        25: stream(b"q 1 1 1 rg 0 0 140 25 re f 0 0 0 RG 0.5 w 1 1 138 23 re S BT /F1 10 Tf 1 0 0 1 5 8 Tm (FORM KEEP) Tj ET Q\n", b" /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 140 25] /Resources 26 0 R"),
        26: b"<< /Font << /F1 8 0 R >> >>",
        28: stream(b"UNREFERENCED SENTINEL: preserve encoded and decoded payload.\n", b" /QASentinel true", True),
    }


def trailer(size, prev=None):
    data = b"<< /Size " + str(size).encode() + b" /Root 1 0 R /Info 15 0 R /ID [<00112233445566778899aabbccddeeff> <ffeeddccbbaa99887766554433221100>] /QAExtension << /Version 1 /Label (retain trailer) >>"
    if prev is not None:
        data += b" /Prev " + str(prev).encode()
    return data + b" >>"


def startxref(data):
    matches = re.findall(rb"startxref\s+(\d+)\s+%%EOF", data)
    if not matches:
        raise ValueError("missing classic startxref")
    return int(matches[-1])


def write_pdf(objects, source=None):
    """Classic-xref local fixture writer; no production export implementation."""
    output = bytearray(source if source is not None else b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    if source is not None:
        output += b"\n"
    offsets = {}
    for number, body in sorted(objects.items()):
        offsets[number] = len(output)
        output += f"{number} 0 obj\n".encode() + body + b"\nendobj\n"
    xref = len(output)
    output += b"xref\n"
    if source is None:
        size = max(objects) + 1
        output += f"0 {size}\n0000000000 65535 f \n".encode()
        for number in range(1, size):
            output += (f"{offsets[number]:010d} 00000 n \n" if number in offsets else "0000000000 00000 f \n").encode()
    else:
        size_match = re.search(rb"/Size\s+(\d+)\b", source[startxref(source):])
        if not size_match:
            raise ValueError("missing source trailer Size")
        size = int(size_match.group(1))
        for number, offset in offsets.items():
            output += f"{number} 1\n{offset:010d} 00000 n \n".encode()
    output += b"trailer\n" + trailer(size, startxref(source) if source is not None else None)
    output += f"\nstartxref\n{xref}\n%%EOF\n".encode()
    return bytes(output)


def generate(directory):
    directory.mkdir(parents=True, exist_ok=True)
    manifest = {"license": "Apache-2.0", "provenance": "Original local fixture generator oracle.py; no downloaded corpus", "fixtures": []}
    for suffix, flate in [("plain", False), ("flate", True)]:
        variants = {f"retained-ascii-{suffix}.pdf": fixture_objects(flate)}
        if not flate:
            fonted = fixture_objects()
            fonted[3] = fonted[3].replace(b"/Resources << >>", b"/Resources 26 0 R")
            fonted[4] = stream(TARGET.replace(b"BT ", b"BT /F1 12 Tf ", 1))
            variants["fonted-refusal.pdf"] = fonted
            shared = fixture_objects()
            shared[10] = shared[10].replace(b"/Contents 11 0 R", b"/Contents 4 0 R")
            variants["shared-owner.pdf"] = shared
            marker = fixture_objects()
            marker[1] = marker[1].replace(b"/AcroForm 23 0 R", b"/AcroForm 23 0 R /QASignature 27 0 R")
            marker[27] = b"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> >>"
            variants["signature-marker.pdf"] = marker
            repeated = fixture_objects()
            repeated[3] = repeated[3].replace(b"/Contents 4 0 R", b"/Contents [4 0 R 4 0 R]")
            variants["repeated-owner.pdf"] = repeated
            foreign = fixture_objects()
            foreign[1] = foreign[1].replace(b"/AcroForm 23 0 R", b"/AcroForm 23 0 R /QAForeign 27 0 R")
            foreign[27] = b"<< /UnknownIncomingTarget 4 0 R >>"
            variants["foreign-owner.pdf"] = foreign
            indirect = fixture_objects()
            indirect[4] = re.sub(rb"/Length \d+", b"/Length 27 0 R", indirect[4], count=1)
            indirect[27] = str(len(TARGET)).encode()
            variants["indirect-length.pdf"] = indirect
            gs = fixture_objects()
            gs[3] = gs[3].replace(b"/Resources << >>", b"/Resources << /ExtGState << /GS1 27 0 R >> >>")
            gs[4] = stream(b"q /GS1 gs " + TARGET + b"Q\n")
            gs[27] = b"<< /Type /ExtGState /ca 0.8 /CA 0.8 >>"
            variants["unsupported-gs.pdf"] = gs
        for name, objects in variants.items():
            path = directory / name
            data = write_pdf(objects)
            path.write_bytes(data)
            manifest["fixtures"].append({"name": name, "sha256": sha(data), "bytes": len(data), "objects": len(objects), "target_object": [4, 0], "target_match_ordinal": 0, "signature_note": "syntactic refusal marker only, not a genuine signed PDF" if name == "signature-marker.pdf" else None})
    # A genuine encrypted PDF, distinct from the syntactic signature marker.
    # RC4-40 is used only as a small compatibility refusal fixture, never security guidance.
    encrypted_source = directory / "fonted-refusal.pdf"
    encrypted = pypdf.PdfWriter()
    encrypted.clone_document_from_reader(pypdf.PdfReader(encrypted_source, strict=True))
    # Cloned textual ID objects are unsuitable for pypdf's encryption hash input.
    encrypted._ID = ArrayObject([ByteStringObject(bytes.fromhex("00112233445566778899aabbccddeeff")), ByteStringObject(bytes.fromhex("ffeeddccbbaa99887766554433221100"))])
    encrypted.encrypt(user_password="qa-open-password", owner_password="qa-owner-password", algorithm="RC4-40")
    encrypted_path = directory / "encrypted-fonted.pdf"
    with encrypted_path.open("wb") as handle:
        encrypted.write(handle)
    encrypted_bytes = encrypted_path.read_bytes()
    encrypted_reader = pypdf.PdfReader(encrypted_path, strict=True)
    assert encrypted_reader.is_encrypted
    assert encrypted_reader.decrypt("qa-open-password")
    assert "ALPHA" in encrypted_reader.pages[0].extract_text()
    manifest["fixtures"].append({"name": encrypted_path.name, "sha256": sha(encrypted_bytes), "bytes": len(encrypted_bytes), "provenance": "pypdf 6.10.0 clone/encrypt of original fonted-refusal.pdf", "encryption": "RC4-40 actual encryption, explicit open refusal control", "target_object": None})
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


def canonical(value):
    """Typed, ordered-independent values; indirect identities remain explicit."""
    if isinstance(value, IndirectObject):
        return {"ref": [value.idnum, value.generation]}
    if isinstance(value, (StreamObject, DictionaryObject)):
        return {"dict": {str(key): canonical(value.raw_get(key)) for key in sorted(value)}}
    if isinstance(value, ArrayObject):
        return {"array": [canonical(item) for item in value]}
    if isinstance(value, NameObject):
        return {"name": str(value)}
    if isinstance(value, TextStringObject):
        return {"string_bytes": value.get_original_bytes().hex()}
    if isinstance(value, ByteStringObject):
        return {"string_bytes": bytes(value).hex()}
    if isinstance(value, BooleanObject):
        return {"bool": value.value}
    if isinstance(value, NullObject):
        return {"null": True}
    if isinstance(value, FloatObject):
        return {"number": float(value)}
    if isinstance(value, NumberObject):
        return {"number": int(value)}
    raise TypeError(f"unsupported independent canonical value {type(value).__name__}")


def raw_body(data, offset):
    match = re.match(rb"(\d+)\s+(\d+)\s+obj\s*", data[offset:])
    if not match:
        raise ValueError("invalid xref object offset")
    begin = offset + match.end()
    marker = re.search(rb"\bstream\r?\n", data[begin:])
    endobj = data.find(b"endobj", begin)
    if marker and begin + marker.start() < endobj:
        header = data[begin:begin + marker.start()]
        length_match = re.search(rb"/Length\s+(\d+)\b", header)
        if not length_match or re.search(rb"/Length\s+\d+\s+\d+\s+R", header):
            raise ValueError("fixture gate requires direct stream /Length")
        payload_start = begin + marker.end()
        payload_end = payload_start + int(length_match.group(1))
        endobj = data.find(b"endobj", payload_end)
    if endobj < begin:
        raise ValueError("missing endobj")
    return data[begin:endobj].rstrip()


def inventory(reader, data):
    if reader.xref_objStm:
        raise ValueError("bounded fixture gate does not admit object streams")
    result, bodies = {}, {}
    for generation, offsets in sorted(reader.xref.items()):
        for number, offset in sorted(offsets.items()):
            if number == 0:
                continue
            ref = IndirectObject(number, generation, reader)
            value = reader.get_object(ref)
            body = raw_body(data, offset)
            key = f"{number}:{generation}"
            record = {"id": [number, generation], "value": canonical(value), "object_body_sha256": sha(body)}
            if isinstance(value, StreamObject):
                header = body.split(b"stream", 1)[0]
                length = int(re.search(rb"/Length\s+(\d+)\b", header).group(1))
                record["value"]["dict"]["/Length"] = {"number": length}
                record["stream"] = {"encoded_sha256": sha(value._data), "decoded_sha256": sha(value.get_data()), "encoded_length": len(value._data), "decoded_length": len(value.get_data())}
                if length != len(value._data):
                    raise ValueError(f"stream length mismatch {key}")
            result[key] = record
            bodies[number] = body
    return result, bodies


def geometry(reader):
    return [{"page_ref": canonical(page.indirect_reference), "MediaBox": canonical(page.mediabox), "CropBox": canonical(page.cropbox), "Rotate": canonical(page.get("/Rotate", NumberObject(0))), "UserUnit": canonical(page.get("/UserUnit", NumberObject(1)))} for page in reader.pages]


def operations(reader):
    content = ContentStream(reader.get_object(IndirectObject(4, 0, reader)), reader)
    return [{"operator": op.decode("ascii"), "operands": [canonical(value) for value in operands]} for operands, op in content.operations]


class LogCapture(logging.Handler):
    def __init__(self):
        super().__init__(logging.WARNING)
        self.messages = []

    def emit(self, record):
        self.messages.append(record.getMessage())


AMBIENT_RENDER_DIAGNOSTICS = "Syntax Error: No display font for 'Symbol'\nSyntax Error: No display font for 'ArialUnicode'\n"
INCREMENTAL_READER_DIAGNOSTIC = "Xref table not zero-indexed. ID numbers for objects will be corrected."


def incremental_reader_control(source, directory):
    """Independent classic-xref append; pin reader diagnostic and mapping."""
    source_reader = pypdf.PdfReader(io.BytesIO(source), strict=True)
    flate = source_reader.get_object(IndirectObject(4, 0, source_reader)).get("/Filter") == "/FlateDecode"
    data = write_pdf({4: stream(EXPECTED, flate=flate)}, source)
    path = directory / "incremental-reader-control.pdf"
    path.write_bytes(data)
    local = LogCapture()
    logger = logging.getLogger("pypdf")
    logger.addHandler(local)
    try:
        reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
        entries, _ = inventory(reader, data)
        expected_ids = {f"{i}:0" for i in list(range(1, 27)) + [28]}
        if set(entries) != expected_ids or reader.get_object(IndirectObject(4, 0, reader)).get_data() != EXPECTED:
            raise RuntimeError("independent incremental control reader mapping/target mismatch")
        offsets = []
        for generation, records in reader.xref.items():
            for number, offset in records.items():
                if number:
                    if not re.match(fr"{number}\s+{generation}\s+obj\b".encode(), data[offset:]):
                        raise RuntimeError("independent incremental control xref id/offset mismatch")
                    offsets.append({"id": [number, generation], "offset": offset})
        if local.messages != [INCREMENTAL_READER_DIAGNOSTIC]:
            raise RuntimeError(f"independent incremental control diagnostic mismatch: {local.messages}")
        result = {"status": "Passed", "qualification": "qualified diagnostic-bearing strict independent parsing, not warning-free/conformance evidence", "file": str(path), "sha256": sha(data), "strict": True, "reader_diagnostics": local.messages, "xref_offsets_verified": sorted(offsets, key=lambda item: item["id"]), "target_operations": operations(reader)}
        (directory / "incremental-reader-control.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        return result
    finally:
        logger.removeHandler(local)


def pdfinfo(path):
    exe = POPPLER / "pdfinfo.exe"
    version = subprocess.run([str(exe), "-v"], capture_output=True, text=True, check=False)
    if version.returncode or "pdfinfo version 26.07.0" not in version.stdout + version.stderr:
        raise RuntimeError("required pinned pdfinfo 26.07.0 unavailable")
    command = [str(exe), "-box", "-f", "1", "-l", "3", str(path)]
    run = subprocess.run(command, capture_output=True, text=True, check=False)
    evidence = {"command": command, "exit_code": run.returncode, "stdout": run.stdout, "stderr": run.stderr, "version": (version.stdout + version.stderr).strip()}
    if run.returncode or run.stderr:
        raise RuntimeError(f"independent Poppler parser failed or warned: {evidence}")
    geometry_lines = [line.strip() for line in run.stdout.splitlines() if re.match(r"^(Pages:|Page\s+\d+\s+(size:|rot:|MediaBox:|CropBox:|BleedBox:|TrimBox:|ArtBox:))", line)]
    evidence["page_geometry_lines"] = geometry_lines
    if not geometry_lines or not any(re.match(r"^Pages:\s+3$", line) for line in geometry_lines):
        raise RuntimeError(f"required pdfinfo three-page geometry missing: {evidence}")
    return evidence


def render(path, prefix, pages=(2, 3), expected_stderr=AMBIENT_RENDER_DIAGNOSTICS):
    exe = POPPLER / "pdftoppm.exe"
    version = subprocess.run([str(exe), "-v"], capture_output=True, text=True, check=False)
    if version.returncode or "pdftoppm version 26.07.0" not in version.stderr + version.stdout:
        raise RuntimeError("required pinned Poppler 26.07.0 unavailable")
    command = [str(exe), "-png", "-f", str(min(pages)), "-l", str(max(pages)), "-r", "96", "-cropbox", "-aa", "yes", "-aaVector", "yes", "-thinlinemode", "none", "-freetype", "yes", str(path), str(prefix)]
    run = subprocess.run(command, capture_output=True, text=True, check=False)
    evidence = {"command": command, "exit_code": run.returncode, "stdout": run.stdout, "stderr": run.stderr, "version": (version.stderr + version.stdout).strip(), "pages": []}
    if run.returncode or run.stdout or (expected_stderr is not None and run.stderr != expected_stderr):
        raise RuntimeError(f"Poppler unaffected-page render failed or warned: {evidence}")
    for number in pages:
        image_path = Path(f"{prefix}-{number}.png")
        if not image_path.is_file():
            raise RuntimeError(f"missing rendered page {number}")
        with Image.open(image_path) as image:
            evidence["pages"].append({"pdf_page": number, "file": str(image_path), "dimensions": list(image.size), "mode": image.mode, "file_sha256": sha(image_path.read_bytes()), "pixels_sha256": sha(image.tobytes())})
    return evidence


def renderer_baseline(directory):
    """Independent valid controls characterize exact ambient diagnostics."""
    controls = []
    for name, content, resources, basefont in [
        ("blank", b"", b"<< >>", b"Helvetica"),
        ("fonted-helvetica", b"BT /F1 14 Tf 1 0 0 1 20 80 Tm (VALID FONTED CONTROL) Tj ET\n", b"<< /Font << /F1 5 0 R >> >>", b"Helvetica"),
        ("fonted-courier", b"BT /F1 14 Tf 1 0 0 1 20 80 Tm (VALID FONTED CONTROL) Tj ET\n", b"<< /Font << /F1 5 0 R >> >>", b"Courier"),
    ]:
        objects = {
            1: b"<< /Type /Catalog /Pages 2 0 R >>",
            2: b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 240 120] /Resources " + resources + b" /Contents 4 0 R >>",
            4: stream(content),
            5: b"<< /Type /Font /Subtype /Type1 /BaseFont /" + basefont + b" /Encoding /WinAnsiEncoding >>",
            15: b"<< /Title (Original independent renderer initialization control) >>",
        }
        path = directory / f"control-{name}.pdf"
        data = write_pdf(objects)
        path.write_bytes(data)
        reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
        assert len(reader.pages) == 1
        assert reader.pages[0].extract_text().strip() == ("" if name == "blank" else "VALID FONTED CONTROL")
        evidence = render(path, directory / f"control-{name}", pages=(1,), expected_stderr=None)
        evidence.update({"control": name, "source_file": str(path), "source_sha256": sha(data), "valid_fonted_or_blank_control": True})
        controls.append(evidence)
    diagnostics = [control["stderr"] for control in controls]
    evidence = {"status": "Failed", "qualification": "diagnostic-bearing renderer comparison; not warning-free/conformance evidence", "inference": "Empty control is diagnostic-free while independently valid Helvetica and Courier controls emit identical two messages; consistent with lazy ambient font initialization", "pinned_ambient_stderr": AMBIENT_RENDER_DIAGNOSTICS, "controls": controls}
    (directory / "renderer-baseline.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    if diagnostics != ["", AMBIENT_RENDER_DIAGNOSTICS, AMBIENT_RENDER_DIAGNOSTICS]:
        raise RuntimeError(f"renderer ambient diagnostic characterization failed: {controls}")
    evidence["status"] = "Passed"
    (directory / "renderer-baseline.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    return evidence


def verify(source, output, mode, report_path, render_only=False):
    report = {"status": "Failed", "scope": "P3a exact local retained ASCII fixture; no universal compatibility claim", "oracle_runtime": {"python_executable": sys.executable, "python_version": sys.version, "pypdf_version": pypdf.__version__, "pypdf_module": pypdf.__file__, "Pillow_version": PIL.__version__}, "source": str(source), "output": str(output), "mode": mode, "target_appearance": {"status": "NotChecked", "reason": "P2b admitted target intentionally has no selected font; target rendering semantics are undefined"}, "checks": [], "failures": []}
    report_path.parent.mkdir(parents=True, exist_ok=True)
    capture = LogCapture()
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
            report["source_sha256"], report["output_sha256"] = sha(a), sha(b)
            original = pypdf.PdfReader(io.BytesIO(a), strict=True)
            source_warnings = list(capture.messages)
            start = len(capture.messages)
            candidate = pypdf.PdfReader(io.BytesIO(b), strict=True)
            candidate_warnings = capture.messages[start:]
            control_warnings = []
            if mode == "Incremental":
                control_dir = report_path.parent / (report_path.stem + "-parse")
                control_dir.mkdir(parents=True, exist_ok=True)
                start = len(capture.messages)
                report["incremental_reader_control"] = incremental_reader_control(a, control_dir)
                control_warnings = capture.messages[start:]
            report["parser_phase_warnings"] = {"source": source_warnings, "candidate_latest_reader": candidate_warnings, "independent_incremental_control": control_warnings}
            before, _ = inventory(original, a)
            after, _ = inventory(candidate, b)
            report["inventory_before"], report["inventory_after"] = before, after
            report["geometry_before"], report["geometry_after"] = geometry(original), geometry(candidate)
            report["operations_before"], report["operations_after"] = operations(original), operations(candidate)
            if not render_only:
                check("exact trusted deterministic source fixture bytes", a in [write_pdf(fixture_objects(False)), write_pdf(fixture_objects(True))])
                check("source fixture object identities", set(before) == {f"{i}:0" for i in list(range(1, 27)) + [28]})
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
                original_stream = original.get_object(IndirectObject(4, 0, original)).get_data()
                candidate_stream = candidate.get_object(IndirectObject(4, 0, candidate)).get_data()
                check("exact source target decoded bytes", original_stream == TARGET)
                check("exact replacement target decoded bytes", candidate_stream == EXPECTED)
                expected_reader = pypdf.PdfReader(io.BytesIO(write_pdf({**fixture_objects(), 4: stream(EXPECTED)})), strict=True)
                check("independent ContentStream exact operators and operands", report["operations_after"] == operations(expected_reader))
                report["trailer_before"], report["trailer_after"] = canonical(original.trailer), canonical(candidate.trailer)
                ta, tb = dict(report["trailer_before"]["dict"]), dict(report["trailer_after"]["dict"])
                pa, pb = ta.pop("/Prev", None), tb.pop("/Prev", None)
                check("all retained trailer values including ID Info Root Size", ta == tb and ta["/Size"] == {"number": 29})
                expected_prev = {"number": startxref(a)} if mode == "Incremental" else None
                check("exact prescribed Prev", pa is None and pb == expected_prev)
                revisions = len(re.findall(rb"startxref\s+\d+\s+%%EOF", b))
                check("exact prescribed revision count", revisions == (2 if mode == "Incremental" else 1), revisions)
                if mode == "Incremental":
                    check("exact original byte prefix", b.startswith(a))
                check("input valid fonted unaffected text", "UNTOUCHED PAGE ONE" in original.pages[1].extract_text() and "UNTOUCHED PAGE TWO" in original.pages[2].extract_text())
                check("unaffected fonted extracted text", [page.extract_text() for page in original.pages[1:]] == [page.extract_text() for page in candidate.pages[1:]])
            render_dir = report_path.parent / (report_path.stem + "-render")
            render_dir.mkdir(parents=True, exist_ok=True)
            if len(original.pages) == len(candidate.pages) == 3:
                report["pdfinfo_before"], report["pdfinfo_after"] = pdfinfo(source), pdfinfo(output)
                check("independent Poppler parser warning-free page geometry", report["pdfinfo_before"]["page_geometry_lines"] == report["pdfinfo_after"]["page_geometry_lines"])
                report["renderer_baseline"] = renderer_baseline(render_dir)
                report["render_before"] = render(source, render_dir / "source")
                report["render_after"] = render(output, render_dir / "output")
                check("exact characterized renderer ambient diagnostics", report["render_before"]["stderr"] == report["render_after"]["stderr"] == report["renderer_baseline"]["pinned_ambient_stderr"])
                left, right = report["render_before"]["pages"], report["render_after"]["pages"]
                check("unaffected valid page render dimensions and exact pixels", [(p["dimensions"], p["mode"], p["pixels_sha256"]) for p in left] == [(p["dimensions"], p["mode"], p["pixels_sha256"]) for p in right])
            else:
                report["render_status"] = "NotChecked: page loss already detected; required untouched pages unavailable"
                check("required unaffected render pages present", False)
            warning_messages = capture.messages + [str(item.message) for item in caught]
            report["parser_warnings"] = warning_messages
            expected = [INCREMENTAL_READER_DIAGNOSTIC] if mode == "Incremental" else []
            check("source independent parser warning-free", not source_warnings)
            check("exact phase-qualified independent parser diagnostics", candidate_warnings == expected and control_warnings == expected and capture.messages == expected + expected and not caught)
            report["parser_qualification"] = "qualified diagnostic-bearing strict parsing" if mode == "Incremental" else "warning-free strict parsing"
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
    reader = pypdf.PdfReader(source, strict=True)
    _, objects = inventory(reader, source.read_bytes())
    flate = b"/FlateDecode" in objects[4]
    good = {**objects, 4: stream(EXPECTED, flate=flate)}
    mutations = {
        "wrong-target": {4: stream(EXPECTED.replace(b"OMEGA", b"SIGMA"), flate=flate)},
        "collateral-target-text": {4: stream(EXPECTED.replace(b"KEEP", b"LOST"), flate=flate)},
        "crop-box": {5: objects[5].replace(b"[10 15 280 205]", b"[11 15 280 205]")},
        "metadata": {14: stream(b"changed metadata", b" /Type /Metadata /Subtype /XML")},
        "attachment": {22: stream(b"Corrupted retained attachment.\n", b" /Type /EmbeddedFile /Subtype /text#2Fplain")},
        "dropped-page": {2: objects[2].replace(b"[3 0 R 5 0 R 10 0 R] /Count 3", b"[3 0 R 5 0 R] /Count 2")},
        "resource-font": {8: objects[8].replace(b"/Helvetica", b"/Times-Roman")},
        "image-pixels": {9: stream(bytes([0] * 12), b" /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", True)},
        "unreferenced-sentinel-change": {28: stream(b"CORRUPTED unreferenced sentinel.\n", b" /QASentinel true", True)},
        "trailer-id": {},
    }
    results = []
    intended_failures = {
        "wrong-target": "exact replacement target decoded bytes",
        "collateral-target-text": "independent ContentStream exact operators and operands",
        "crop-box": "page geometry and page tree",
        "metadata": "all non-target dictionaries and stream payloads",
        "attachment": "all non-target dictionaries and stream payloads",
        "dropped-page": "page geometry and page tree",
        "resource-font": "all non-target dictionaries and stream payloads",
        "image-pixels": "all non-target dictionaries and stream payloads",
        "unreferenced-sentinel-change": "all non-target dictionaries and stream payloads",
        "trailer-id": "all retained trailer values including ID Info Root Size",
    }
    for name, mutation in mutations.items():
        modified = {**good, **mutation}
        if mode == "Incremental":
            replacements = {number: body for number, body in modified.items() if body != objects[number]}
            data = write_pdf(replacements, source.read_bytes())
        else:
            data = write_pdf(modified)
        if name == "trailer-id":
            # For Incremental, mutate only the new trailer, retaining original prefix.
            pos = data.rfind(b"00112233445566778899aabbccddeeff")
            data = data[:pos] + b"10112233445566778899aabbccddeeff" + data[pos + 32:]
        mutant = directory / f"{name}.pdf"
        mutant.write_bytes(data)
        result = verify(source, mutant, mode, directory / f"{name}.json")
        intended = intended_failures[name]
        results.append({"name": name, "mutant": str(mutant), "sha256": sha(data), "intended_failure": intended, "rejected": result["status"] == "Failed" and not result.get("exception") and intended in result["failures"], "failures": result["failures"], "exception": result.get("exception")})
    if mode == "Compact":
        omitted = {number: body for number, body in good.items() if number != 28}
        mutant = directory / "unreferenced-sentinel-omit.pdf"
        mutant.write_bytes(write_pdf(omitted))
        result = verify(source, mutant, mode, directory / "unreferenced-sentinel-omit.json")
        intended = "retained object identities and generations"
        results.append({"name": "unreferenced-sentinel-omit", "mutant": str(mutant), "sha256": sha(mutant.read_bytes()), "intended_failure": intended, "rejected": result["status"] == "Failed" and not result.get("exception") and intended in result["failures"], "failures": result["failures"], "exception": result.get("exception")})
    # Render isolation proves the rendering comparator itself rejects changed pixels,
    # even when inventory failures are deliberately outside this one negative check.
    render_negative = verify(source, directory / "image-pixels.pdf", mode, directory / "render-only-image.json", render_only=True)
    results.append({"name": "render-only-image", "intended_failure": "unaffected valid page render dimensions and exact pixels", "rejected": render_negative["status"] == "Failed" and not render_negative.get("exception") and "unaffected valid page render dimensions and exact pixels" in render_negative["failures"], "failures": render_negative["failures"], "exception": render_negative.get("exception")})
    summary = {"status": "Passed" if positive["status"] == "Passed" and all(item["rejected"] for item in results) else "Failed", "positive_status": positive["status"], "mutants": results, "target_appearance": "NotChecked: intentionally fontless P2b target", "scope": "bounded oracle self-test; mutations are local synthetic outputs, not production exports"}
    (directory / "self-test.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    gen = sub.add_parser("generate")
    gen.add_argument("directory", nargs="?", type=Path, default=HERE.parent.parent / "crates" / "binas-pdf" / "tests" / "fixtures" / "retained-ascii")
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
    if args.command == "verify":
        result = verify(args.source, args.output, args.mode, args.report)
    else:
        result = self_test(args.source, args.output, args.mode, args.directory)
    print(json.dumps({key: result.get(key) for key in ["status", "failures", "exception", "positive_status", "mutants"] if key in result}, indent=2))
    return 0 if result["status"] == "Passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
