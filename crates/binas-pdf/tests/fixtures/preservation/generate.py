# SPDX-License-Identifier: Apache-2.0
"""Generate original, tiny PDF boundary fixtures and their mandatory manifest.

Run explicitly when updating fixtures; tests consume checked-in bytes, never regenerate.
These are not copied competitor reproductions or broad compatibility evidence.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def stream(data):
    return b"<< /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"


def pdf(objects, trailer=""):
    result = bytearray(b"%PDF-1.7\n")
    offsets = []
    for number, obj in enumerate(objects, 1):
        offsets.append(len(result))
        result.extend(f"{number} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(result)
    result.extend(f"xref\n0 {len(objects)+1}\n0000000000 65535 f \n".encode())
    for offset in offsets:
        result.extend(f"{offset:010} 00000 n \n".encode())
    result.extend(f"trailer\n<< /Size {len(objects)+1} /Root 1 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(result)


def base(extra=""):
    return [
        f"<< /Type /Catalog /Pages 2 0 R {extra} >>".encode(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        stream(b"BT /F1 12 Tf 20 100 Td (A) Tj ET"),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]


fixtures = []


def emit(name, objects, keys=(), trailer="", trailer_keys=(), effect="refuse_unrequested_loss"):
    data = pdf(objects, trailer)
    (ROOT / name).write_bytes(data)
    fixtures.append(dict(path=name, sha256=hashlib.sha256(data).hexdigest(),
                         origin="original_generated", license="Apache-2.0", mandatory=True,
                         expected_effect=effect, catalog_entries=list(keys), trailer_entries=list(trailer_keys)))


emit("metadata.pdf", base("/Metadata 6 0 R") + [
    stream(b'<x:xmpmeta xmlns:x="adobe:ns:meta/">Original metadata</x:xmpmeta>').replace(b'<< /Length', b'<< /Type /Metadata /Subtype /XML /Length', 1),
    b"<< /Title (Original title) >>"], ["Metadata"], "/Info 7 0 R", ["Info"])
emit("outlines.pdf", base("/Outlines 6 0 R") + [
    b"<< /Type /Outlines /First 7 0 R /Last 7 0 R /Count 1 >>",
    b"<< /Title (Original bookmark) /Parent 6 0 R /Dest [3 0 R /Fit] >>"], ["Outlines"])
emit("names.pdf", base("/Names 6 0 R") + [
    b"<< /Dests << /Names [(start) [3 0 R /Fit]] >> >>"], ["Names"])
emit("labels.pdf", base("/PageLabels 6 0 R") + [
    b"<< /Nums [0 << /S /D /P (A-) /St 1 >>] >>"], ["PageLabels"])
tagged = base("/StructTreeRoot 6 0 R /MarkInfo << /Marked true >>")
tagged[2] = tagged[2][:-2] + b" /StructParents 0 >>"
tagged[3] = stream(b"/Figure << /MCID 0 >> BDC 0 0 20 20 re f EMC")
alt = "Imagen: café 中文 😀".encode("utf-16-be").hex().upper()
tagged += [b"<< /Type /StructTreeRoot /K [7 0 R] /ParentTree 8 0 R /ParentTreeNextKey 1 >>",
           f"<< /Type /StructElem /S /Figure /P 6 0 R /Pg 3 0 R /K 0 /Alt <FEFF{alt}> >>".encode(),
           b"<< /Nums [0 [7 0 R]] >>"]
emit("unicode-alt-tags.pdf", tagged, ["StructTreeRoot", "MarkInfo"])

for shared in (True, False):
    font = b"<< /Type /Font /Subtype /Type3 /Name /F1 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 9 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [600] /Resources << >> >>"
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>",
               b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>",
               b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources 7 0 R >>",
               f"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources {7 if shared else 10} 0 R >>".encode(),
               stream(b"BT /F1 12 Tf 20 100 Td (A) Tj ET"),
               stream(b"BT /F1 12 Tf 20 100 Td (AA) Tj ET"),
               b"<< /Font << /F1 8 0 R >> >>", font,
               stream(b"600 0 0 0 600 700 d1 0 0 600 700 re f")]
    if not shared:
        objects += [b"<< /Font << /F1 11 0 R >> >>", font]
    emit("type3-shared.pdf" if shared else "type3-separate.pdf", objects,
         effect="extract_reversed_pages_preserves_text_and_glyph_stream")

(ROOT / "manifest.json").write_text(json.dumps(dict(schema_version=1, fixtures=fixtures), indent=2) + "\n", encoding="utf-8")
