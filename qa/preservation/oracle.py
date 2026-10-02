"""Original Apache-2.0 reorder fixture and independent pypdf oracle.

Requires exactly pypdf 6.10.0. Missing input/tools and any failed check are fatal.
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
if not __debug__:
    raise RuntimeError("Run without -O: assertion checks are mandatory")
assert pypdf.__version__ == "6.10.0", "Install the pinned requirements.txt"
WARNINGS = []


class CaptureWarnings(logging.Handler):
    def emit(self, record):
        if record.levelno >= logging.WARNING:
            WARNINGS.append(record.getMessage())


logging.getLogger("pypdf").addHandler(CaptureWarnings())


def sha(data):
    return hashlib.sha256(data).hexdigest()


def stream(data, extra=b""):
    return b"<< /Length " + str(len(data)).encode() + b" " + extra + b" >>\nstream\n" + data + b"\nendstream"


def text(value):
    return b"<FEFF" + value.encode("utf-16-be").hex().upper().encode() + b">"


def objects():
    o = {
        1: b"<< /Type /Catalog /Pages 7 0 R /Metadata 30 0 R /Outlines 20 0 R /Names 22 0 R /PageLabels 23 0 R /AcroForm 24 0 R /StructTreeRoot 26 0 R /MarkInfo << /Marked true >> /Lang (en-US) >>",
        2: b"<< /Type /Pages /Parent 7 0 R /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [0 0 300 400] /CropBox [10 20 290 380] /Rotate 90 /Resources 11 0 R >>",
        3: b"<< /Type /Pages /Parent 7 0 R /Kids [6 0 R] /Count 1 /MediaBox [0 0 500 600] /CropBox [30 40 470 560] /Rotate 270 /Resources 11 0 R >>",
        4: b"<< /Type /Page /Parent 2 0 R /Contents 8 0 R /StructParents 0 /Annots [25 0 R 35 0 R] >>",
        5: b"<< /Type /Page /Parent 2 0 R /Contents 9 0 R /StructParents 1 >>",
        6: b"<< /Type /Page /Parent 3 0 R /Contents 10 0 R /StructParents 2 >>",
        7: b"<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 3 >>",
        11: b"<< /Font << /F1 12 0 R /T3 13 0 R >> /XObject << /Fm 15 0 R /Im 16 0 R >> >>",
        12: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        13: b"<< /Type /Font /Subtype /Type3 /Name /T3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 14 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [600] /Resources << >> /ToUnicode 17 0 R >>",
        14: stream(b"600 0 0 0 600 700 d1 0 0 600 700 re f"),
        15: stream(b"q 20 0 0 20 0 0 cm /Im Do Q", b"/Type /XObject /Subtype /Form /BBox [0 0 20 20] /Resources << /XObject << /Im 16 0 R >> >>"),
        16: stream(bytes([255, 0, 0]), b"/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8"),
        17: stream(b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /OriginalType3 def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <41> <0041> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end"),
        20: b"<< /Type /Outlines /First 21 0 R /Last 21 0 R /Count 1 >>",
        21: b"<< /Title " + text("R\u00e9sum\u00e9 \u6771\u4eac") + b" /Parent 20 0 R /Dest [6 0 R /Fit] >>",
        22: b"<< /Dests << /Names [(landing) [5 0 R /Fit]] >> /EmbeddedFiles << /Names [(note.txt) 32 0 R] >> >>",
        23: b"<< /Nums [0 << /S /D /P (A-) /St 1 >> 1 << /S /D /P (B-) /St 1 >> 2 << /S /D /P (C-) /St 1 >>] >>",
        24: b"<< /Fields [25 0 R] /DA (/F1 12 Tf 0 g) /DR << /Font << /F1 12 0 R >> >> /CO [25 0 R] >>",
        25: b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (customer) /V " + text("Zo\u00eb") + b" /Rect [30 40 150 60] /P 4 0 R /F 4 /AP << /N 34 0 R >> /StructParent 3 >>",
        26: b"<< /Type /StructTreeRoot /K [27 0 R 28 0 R 29 0 R 37 0 R] /ParentTree 36 0 R /ParentTreeNextKey 4 >>",
        27: b"<< /Type /StructElem /S /P /P 26 0 R /Pg 4 0 R /K 0 /Alt " + text("First \u6771\u4eac") + b" >>",
        28: b"<< /Type /StructElem /S /P /P 26 0 R /Pg 5 0 R /K 0 >>",
        29: b"<< /Type /StructElem /S /P /P 26 0 R /Pg 6 0 R /K 0 >>",
        30: stream(('<?xpacket begin="\ufeff"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">R\u00e9sum\u00e9 \u6771\u4eac</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>').encode(), b"/Type /Metadata /Subtype /XML"),
        31: b"<< /Title " + text("R\u00e9sum\u00e9 \u6771\u4eac") + b" /Author (Binas original QA fixture) >>",
        32: b"<< /Type /Filespec /F (note.txt) /UF " + text("note.txt") + b" /EF << /F 33 0 R >> /Desc (Original QA attachment) >>",
        33: stream(b"Original attachment: preserve these exact bytes.\n", b"/Type /EmbeddedFile /Subtype /text#2Fplain"),
        34: stream(b"BT /F1 12 Tf 2 4 Td (Zoe) Tj ET", b"/Type /XObject /Subtype /Form /BBox [0 0 120 20] /Resources << /Font << /F1 12 0 R >> >>"),
        35: b"<< /Type /Annot /Subtype /Link /Rect [30 80 150 100] /P 4 0 R /Dest [6 0 R /Fit] /Border [0 0 0] >>",
        36: b"<< /Nums [0 [27 0 R] 1 [28 0 R] 2 [29 0 R] 3 37 0 R] >>",
        37: b"<< /Type /StructElem /S /Form /P 26 0 R /Pg 4 0 R /K << /Type /OBJR /Obj 25 0 R /Pg 4 0 R >> >>",
    }
    for number, name in [(8, "ALPHA"), (9, "BRAVO"), (10, "CHARLIE")]:
        o[number] = stream(f"/P << /MCID 0 >> BDC BT /F1 12 Tf 30 100 Td ({name}) Tj /T3 12 Tf (A) Tj ET EMC q 1 0 0 1 50 150 cm /Fm Do Q q 10 0 0 10 80 150 cm /Im Do Q".encode())
    return o


def serialize(o):
    data = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for number, body in sorted(o.items()):
        offsets[number] = len(data)
        data.extend(f"{number} 0 obj\n".encode() + body + b"\nendobj\n")
    xref = len(data)
    data.extend(f"xref\n0 {max(o)+1}\n0000000000 65535 f \n".encode())
    for number in range(1, max(o)+1):
        data.extend(f"{offsets[number]:010} 00000 n \n".encode() if number in o else b"0000000000 00000 f \n")
    data.extend(f"trailer\n<< /Size {max(o)+1} /Root 1 0 R /Info 31 0 R /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>] >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(data)


def canonical(value):
    if isinstance(value, IndirectObject):
        return ["ref", value.idnum, value.generation]
    if isinstance(value, StreamObject):
        return {"dict": {str(k): canonical(v) for k, v in sorted(value.items()) if k != "/Length"}, "decoded_sha256": sha(value.get_data())}
    if isinstance(value, DictionaryObject):
        return {str(k): canonical(v) for k, v in sorted(value.items())}
    if isinstance(value, (list, ArrayObject)):
        return [canonical(v) for v in value]
    if isinstance(value, bytes):
        return ["bytes", value.hex()]
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    return str(value)


def inventory(data, label_reader=None):
    reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
    pages = list(reader.pages)
    result = {"pages": [], "labels": reader.page_labels if label_reader is None else label_reader(reader), "objects": {}}
    for page in pages:
        result["pages"].append({"id": page.indirect_reference.idnum, "media": list(page.mediabox), "crop": list(page.cropbox), "rotate": page.get("/Rotate", 0), "resources": canonical(page["/Resources"]), "text": page.extract_text(), "annots": canonical(page.get("/Annots", [])), "struct_parents": page.get("/StructParents")})
    # Exact source object identities are part of this full-reorder contract.
    # Exclude only topology, pages and page-label number tree, checked separately.
    for number in objects():
        if number not in {2, 3, 4, 5, 6, 7, 23}:
            value = canonical(reader.get_object(number))
            if number == 1:
                # Reorder may rewrite topology/label storage while retaining
                # equivalent page occurrences and labels, checked above/below.
                value = {k: v for k, v in value.items() if k not in {"/Pages", "/PageLabels"}}
            result["objects"][str(number)] = value
    result["info"] = canonical(reader.trailer.raw_get("/Info"))
    result["trailer_id"] = canonical(reader.trailer["/ID"])
    result["fields"] = sorted(reader.get_fields())
    result["named_destinations"] = {key: reader.get_destination_page_number(value) for key, value in reader.named_destinations.items()}
    result["outline_destinations"] = [reader.get_destination_page_number(item) for item in reader.outline]
    result["attachments"] = {name: [sha(item) for item in values] for name, values in reader.attachments.items()}
    return result


def verify(source, candidate, order, label_reader=None):
    before, after = inventory(source, label_reader), inventory(candidate, label_reader)
    assert after["pages"] == [before["pages"][i] for i in order], "page identity/text/geometry/resources/annotation/tag inventory changed"
    assert after["labels"] == [before["labels"][i] for i in order], "labels did not follow source pages"
    for key in ["objects", "info", "trailer_id", "fields", "attachments"]:
        assert before[key] == after[key], f"preservation mismatch: {key}"
    assert after["named_destinations"] == {k: order.index(v) for k, v in before["named_destinations"].items()}, "named destination mapping changed"
    assert after["outline_destinations"] == [order.index(v) for v in before["outline_destinations"]], "outline mapping changed"
    return after


def source_checks(data):
    reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
    root = reader.trailer["/Root"]
    assert root["/Metadata"].get_data().decode("utf-8").find("R\u00e9sum\u00e9 \u6771\u4eac") >= 0
    assert reader.metadata.title == "R\u00e9sum\u00e9 \u6771\u4eac"
    structure = root["/StructTreeRoot"]
    assert [item.idnum for item in structure["/K"]] == [27, 28, 29, 37]
    parent_tree = structure["/ParentTree"]["/Nums"]
    for index, page in enumerate(reader.pages):
        assert page["/StructParents"] == index
        assert parent_tree[index * 2] == index
        assert parent_tree[index * 2 + 1][0].idnum == 27 + index
        elem = reader.get_object(27 + index)
        assert elem.raw_get("/Pg").idnum == page.indirect_reference.idnum
        assert elem["/K"] == 0
        assert b"/MCID 0" in page.get_contents().get_data()
        assert page["/Resources"]["/Font"].raw_get("/T3").idnum == 13
        assert page["/Resources"]["/XObject"].raw_get("/Fm").idnum == 15
        assert page["/Resources"]["/XObject"].raw_get("/Im").idnum == 16
    assert reader.get_object(15)["/Resources"]["/XObject"].raw_get("/Im").idnum == 16
    widget = reader.get_object(25)
    assert widget.raw_get("/P").idnum == 4
    assert widget["/StructParent"] == 3
    assert parent_tree[7].idnum == 37
    assert reader.get_object(37)["/K"].raw_get("/Obj").idnum == 25
    assert root["/AcroForm"]["/Fields"][0].idnum == 25
    assert reader.get_object(4)["/Annots"][0].idnum == 25
    assert widget["/V"] == "Zo\u00eb"
    inv = verify(data, data, [0, 1, 2])
    assert [p["id"] for p in inv["pages"]] == [4, 5, 6]
    assert inv["labels"] == ["A-1", "B-1", "C-1"]
    assert inv["fields"] == ["customer"]
    assert inv["named_destinations"] == {"landing": 1}
    assert inv["outline_destinations"] == [2]
    assert all(token in page["text"] for token, page in zip(["ALPHA", "BRAVO", "CHARLIE"], inv["pages"]))
    assert inv["pages"][0]["media"] != inv["pages"][2]["media"]
    assert inv["pages"][0]["rotate"] != inv["pages"][2]["rotate"]
    return inv


def negative_checks(source):
    corruptions = {
        "bookmark": (1, b"/Outlines 20 0 R", b""),
        "named_destination": (22, b"(landing)", b"(missing)"),
        "field": (24, b"/Fields [25 0 R]", b"/Fields []"),
        "tag": (26, b"/K [27 0 R 28 0 R 29 0 R 37 0 R]", b"/K [28 0 R 29 0 R 37 0 R]"),
        "parent_tree": (36, b"0 [27 0 R]", b"0 [28 0 R]"),
        "resource": (11, b"/T3 13 0 R", b"/T3 12 0 R"),
        "attachment": (33, b"preserve", b"MUTATED!"),
        "unicode": (21, text("R\u00e9sum\u00e9 \u6771\u4eac"), text("truncated")),
        "inherited_rotation": (3, b"/Rotate 270", b"/Rotate 90"),
        "page_labels": (1, b"/PageLabels 23 0 R", b""),
        "page_order": (7, b"/Kids [2 0 R 3 0 R]", b"/Kids [3 0 R 2 0 R]"),
    }
    rejected = []
    for name, (number, old, new) in corruptions.items():
        changed = objects()
        assert old in changed[number]
        changed[number] = changed[number].replace(old, new)
        data = serialize(changed)
        # Independent reopen must succeed before a corruption is credited.
        assert len(pypdf.PdfReader(io.BytesIO(data), strict=True).pages) == 3
        try:
            verify(source, data, [0, 1, 2])
        except AssertionError:
            rejected.append(name)
        else:
            raise AssertionError(f"oracle accepted corruption: {name}")
    return rejected


def positive_control(source):
    # Independent, explicit synthetic transformation confirms the comparator
    # accepts the intended reorder; this is not a Binas-produced candidate.
    changed = objects()
    changed[7] = b"<< /Type /Pages /Kids [6 0 R 4 0 R 5 0 R] /Count 3 >>"
    for page in [4, 5, 6]:
        parent = 2 if page in [4, 5] else 3
        properties = (b"/MediaBox [0 0 300 400] /CropBox [10 20 290 380] /Rotate 90" if parent == 2 else b"/MediaBox [0 0 500 600] /CropBox [30 40 470 560] /Rotate 270")
        changed[page] = changed[page].replace(f"/Parent {parent} 0 R".encode(), b"/Parent 7 0 R " + properties + b" /Resources 11 0 R")
    changed[23] = b"<< /Nums [0 << /S /D /P (C-) /St 1 >> 1 << /S /D /P (A-) /St 1 >> 2 << /S /D /P (B-) /St 1 >>] >>"
    verify(source, serialize(changed), [2, 0, 1])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["generate", "source", "verify"])
    parser.add_argument("--candidate", type=Path)
    args = parser.parse_args()
    fixture = ROOT / "fixtures/reorder-structures.pdf"
    if args.command == "generate":
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_bytes(serialize(objects()))
        alpha = objects()
        alpha[23] = b"<< /Nums [0 << /S /A /St 26 >>] >>"
        alpha_path = ROOT / "fixtures/alpha-boundary.pdf"
        alpha_path.write_bytes(serialize(alpha))
        (ROOT / "manifest.json").write_text(json.dumps({"schema": 1, "license": "Apache-2.0", "origin": "Original Binas QA synthetic fixtures generated by oracle.py; no third-party PDF content", "required": [{"path": "fixtures/reorder-structures.pdf", "sha256": sha(fixture.read_bytes())}, {"path": "fixtures/alpha-boundary.pdf", "sha256": sha(alpha_path.read_bytes())}], "requested_order": [2, 0, 1], "source_page_objects": [4, 5, 6]}, indent=2)+"\n")
    source = fixture.read_bytes()
    manifest = json.loads((ROOT / "manifest.json").read_text())
    assert sha(source) == manifest["required"][0]["sha256"], "fixture hash mismatch"
    before = source_checks(source)
    rejected = negative_checks(source)
    positive_control(source)
    receipt = {"pypdf": pypdf.__version__, "python": platform.python_version(), "source_sha256": sha(source), "oracle_sha256": sha(Path(__file__).read_bytes()), "source_validated": True, "synthetic_correct_reorder_control": "passed", "reparsable_corruptions_rejected": rejected, "source_inventory": before}
    if args.command == "verify":
        assert args.candidate is not None, "--candidate is required"
        candidate = args.candidate.read_bytes()
        receipt.update(candidate_sha256=sha(candidate), requested_order=manifest["requested_order"], candidate_inventory=verify(source, candidate, manifest["requested_order"]), result="passed")
    else:
        receipt["result"] = "source_and_negative_controls_passed"
    receipt["reader_warnings"] = sorted(set(WARNINGS))
    evidence = ROOT / "evidence"
    evidence.mkdir(exist_ok=True)
    receipt_name = args.candidate.stem + ".json" if args.command == "verify" else "source.json"
    (evidence / receipt_name).write_text(json.dumps(receipt, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in receipt.items() if not k.endswith("inventory")}, indent=2))


if __name__ == "__main__":
    main()
