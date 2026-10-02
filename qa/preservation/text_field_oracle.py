"""Original Apache-2.0 retained text-field fill fixture and independent gate.

Two terminal Tx values change in one revision, followed by reorder [1,0]. Each
selected widget receives a private appearance clone with one replaced text token.
An unselected field keeps the shared original appearance. Fonts, static paint,
clip, geometry, defaults and document structures remain. This fixed Courier
template case is not arbitrary font, shaping, accessibility or layout assurance.
"""
import argparse
import io
import json
import platform
import re
import subprocess
import zlib
from pathlib import Path

import pypdf
from pypdf.generic import (ArrayObject, ByteStringObject, DictionaryObject,
                          IndirectObject, NameObject, NumberObject, StreamObject)

import flate_oracle as storage
import flatten_oracle as flatten
import geometry_oracle as geometry
import oracle

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
SOURCE = ROOT / "fixtures/text-field-fill.pdf"
PROVENANCE = ROOT / "fixtures/text-field-fill.provenance.json"
ORDER = [1, 0]
REQUESTS = {"group.name": "Café (A) \\ B", "code": "Ready (2)"}
FIELDS = {"group.name": {"field": 8, "widgets": [(9, 3), (10, 4)], "appearance": 14, "old": "Old"},
          "code": {"field": 12, "widgets": [(12, 4)], "appearance": 15, "old": "Start"}}
ATTACHMENT = b"Original text-field attachment; retain NUL:\x00 and newline.\n"
sha, require = storage.sha, storage.require


def fixture_bytes():
    stream = storage.stream
    form = b"/Type /XObject /Subtype /Form /FormType 1 /Matrix [1 0 0 1 0 0] /Resources << /Font << /F1 19 0 R >> >>"
    def template(text, width, color):
        return (f"q\n{color} rg\n0 0 {width} 32 re f\n0.15 0.25 0.45 RG\n1 w\n0.5 0.5 {width-1} 31 re S\n2 2 {width-4} 28 re W n\nBT\n/F1 10 Tf\n0 g\n1 0 0 1 6 10 Tm\n({text}) Tj\nET\nQ\n").encode("ascii")
    box = b"q 1 1 1 rg 0 0 24 24 re f 0.15 0.25 0.45 RG 1 w 1 1 22 22 re S Q\n"
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R /AcroForm 23 0 R /Metadata 24 0 R /Outlines 25 0 R /Names 27 0 R /PageLabels 31 0 R /Lang (en-US) /Private (original catalog) >>",
        2: b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 260] /Resources 32 0 R /Private (original page tree) >>",
        3: b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [9 0 R 11 0 R 18 0 R] /Private (alpha page) >>",
        4: b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Annots [10 0 R 12 0 R 13 0 R] /Private (bravo page) >>",
        5: stream(b"BT /F1 19 Tf 30 230 Td (PAGE ALPHA) Tj /F1 11 Tf 0 -94 Td (Unselected shared owner below) Tj ET\n"),
        6: stream(b"BT /F1 19 Tf 30 230 Td (PAGE BRAVO) Tj /F1 11 Tf 0 -94 Td (Second selected field below) Tj ET\n"),
        7: b"<< /T (group) /Kids [8 0 R 11 0 R] /Private (original field group) >>",
        8: b"<< /Parent 7 0 R /FT /Tx /Ff 0 /T (name) /TU (Selected name) /V (Old) /DV (Default name) /Q 0 /MaxLen 24 /Kids [9 0 R 10 0 R] /Private (selected terminal) >>",
        9: b"<< /Type /Annot /Subtype /Widget /Parent 8 0 R /P 3 0 R /Rect [30 165 210 197] /F 4 /AP 33 0 R /Private (selected alpha widget) >>",
        10: b"<< /Type /Annot /Subtype /Widget /Parent 8 0 R /P 4 0 R /Rect [30 165 210 197] /F 4 /AP 33 0 R /Private (selected bravo widget) >>",
        11: b"<< /Type /Annot /Subtype /Widget /Parent 7 0 R /FT /Tx /Ff 0 /T (keep) /TU (Unselected shared appearance) /V (Old) /DV (Old) /Q 0 /P 3 0 R /Rect [30 80 210 112] /F 4 /AP 33 0 R /Private (keep interactive and visually unchanged) >>",
        12: b"<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 0 /T (code) /TU (Selected code) /V (Start) /DV (Default code) /Q 0 /MaxLen 20 /P 4 0 R /Rect [30 80 180 112] /F 4 /AP << /N 15 0 R /Private (direct AP dictionary) >> /Private (combined selected field) >>",
        13: b"<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 0 /T (accept) /V /Yes /DV /Off /AS /Yes /P 4 0 R /Rect [220 50 244 74] /F 4 /AP << /N << /Off 16 0 R /Yes 17 0 R >> >> /Private (retained checkbox) >>",
        14: stream(template("Old", 180, "0.88 0.94 1"), form + b" /BBox [0 0 180 32] /Private << /Label (shared appearance extension) /Data 34 0 R /Flags [1 null /Preserve] >>", True),
        15: stream(template("Start", 150, "0.9 0.97 0.86"), form + b" /BBox [0 0 150 32] /Private << /Label (code appearance extension) /Data 34 0 R >>", True),
        16: stream(box, form + b" /BBox [0 0 24 24]"),
        17: stream(box + b"q 0.05 0.45 0.2 RG 3 w 5 12 m 10 7 l 19 18 l S Q\n", form + b" /BBox [0 0 24 24]"),
        18: b"<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [30 35 200 55] /Dest [4 0 R /Fit] /Border [0 0 1] /Contents (Retained link) >>",
        19: b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 255 /Widths [" + b" ".join([b"600"] * 224) + b"] /FontDescriptor 20 0 R /ToUnicode 22 0 R /Private (appearance font) >>",
        20: b"<< /Type /FontDescriptor /FontName /Courier /Flags 33 /FontBBox [-50 -250 650 900] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 600 /StemV 80 /AvgWidth 600 /MissingWidth 600 >>",
        21: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /Private (different page F1 font) >>",
        22: stream(b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /OriginalCourierWinAnsi def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfrange <20> <7E> <0020> endbfrange 1 beginbfchar <E9> <00E9> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end"),
        23: b"<< /Fields [7 0 R 12 0 R 13 0 R] /NeedAppearances false /DA (/F1 10 Tf 0 g) /DR << /Font << /F1 19 0 R >> >> /Private (original AcroForm) >>",
        24: stream(b'<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Original retained text-field fixture</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>', b"/Type /Metadata /Subtype /XML"),
        25: b"<< /Type /Outlines /First 26 0 R /Last 26 0 R /Count 1 >>",
        26: b"<< /Title (Bravo page) /Parent 25 0 R /Dest [4 0 R /Fit] >>",
        27: b"<< /Dests << /Names [(bravo) [4 0 R /Fit]] >> /EmbeddedFiles << /Names [(note.txt) 28 0 R] >> >>",
        28: b"<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F 29 0 R >> /Desc (Exact original attachment) >>",
        29: stream(ATTACHMENT, b"/Type /EmbeddedFile /Subtype /text#2Fplain"),
        30: b"<< /Title (Original retained text-field fixture) /Author (Binas original Apache-2.0 QA) >>",
        31: b"<< /Nums [0 << /S /D /P (A-) /St 1 /Private (alpha label) >> 1 << /S /D /P (B-) /St 1 /Private (bravo label) >>] /Private (original label tree) >>",
        32: b"<< /Font << /F1 21 0 R >> /ProcSet [/PDF /Text] /Private (inherited page resources) >>",
        33: b"<< /N 14 0 R /Private (shared AP dictionary) /Opaque 34 0 R >>",
        34: b"<< /Meaning (Opaque extension retained exactly) /Bytes <00FF0102> >>",
    }
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for number, value in sorted(objects.items()):
        offsets[number] = len(output)
        output += f"{number} 0 obj\n".encode() + value + b"\nendobj\n"
    xref = len(output)
    output += b"xref\n0 35\n0000000000 65535 f \n"
    for number in range(1, 35):
        output += f"{offsets[number]:010} 00000 n \n".encode()
    output += ("trailer\n<< /Size 35 /Root 1 0 R /Info 30 0 R "
               "/ID [<544558544649454C4446494C4C30303031> <544558544649454C4446494C4C30303031>] >>\n"
               f"startxref\n{xref}\n%%EOF\n").encode()
    return bytes(output)


def literal(encoded):
    return b"(" + encoded.replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)") + b")"


def requested_program(original, old_value, value):
    old_token = literal(old_value.encode("cp1252"))
    require(original.count(old_token + b" Tj") == 1, "original appearance has no unique selected Tj token")
    return original.replace(old_token, literal(value.encode("cp1252")), 1)


def verify_token_replacement(original, candidate, old_value, value, reader):
    """Recognize this frozen template's source token independently of Binas.

    Binas emits canonical uppercase hex. The independent author deliberately
    keeps escaped literal strings. Only these two complete encodings of the
    same requested glyph bytes are accepted; every byte outside the unique
    source Tj string operand remains exact.
    """
    source_token = literal(old_value.encode("cp1252"))
    require(original.count(source_token + b" Tj") == 1,
            "source appearance has no unique literal Tj token boundary")
    source_ops = flatten.operations(original, reader)
    source_strings = [operands[0] for operands, op in source_ops if op == b"Tj"]
    require(len(source_strings) == 1 and source_strings[0].original_bytes == old_value.encode("cp1252"),
            "source Tj token does not encode the original field value")
    start = original.index(source_token + b" Tj")
    end = start + len(source_token)
    prefix, suffix = original[:start], original[end:]
    require(len(candidate) >= len(prefix) + len(suffix)
            and candidate.startswith(prefix) and candidate.endswith(suffix),
            "appearance changed outside the source Tj token boundary")
    token = candidate[len(prefix):len(candidate)-len(suffix)]
    glyphs = value.encode("cp1252")
    encodings = {literal(glyphs): "escaped_literal",
                 b"<" + glyphs.hex().upper().encode("ascii") + b">": "canonical_uppercase_hex"}
    require(token in encodings, "appearance token is not an allowed exact encoding of requested glyph bytes")
    parsed_ops = flatten.operations(candidate, reader)
    strings = [operands[0] for operands, op in parsed_ops if op == b"Tj"]
    require(len(strings) == 1 and strings[0].original_bytes == glyphs,
            "independently parsed appearance glyph bytes disagree with the request")
    return {"source_decoded_token_span": [start, end], "output_token_encoding": encodings[token],
            "preserved_prefix_bytes": len(prefix), "preserved_suffix_bytes": len(suffix)}


def pdf_value(value):
    return value.encode("ascii") if value.isascii() else b"\xfe\xff" + value.encode("utf-16-be")


def same_except(before, after, except_keys):
    require(set(before) == set(after), "unrequested dictionary keys added or removed")
    for key, value in before.items():
        if key not in except_keys:
            require(storage.typed(after.raw_get(key)) == storage.typed(value), f"unrequested dictionary key changed: {key}")


def inspect(data):
    reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
    fields = reader.get_fields()
    require(fields is not None, "active form registration missing")
    return {"fields": {name: {key: storage.typed(value) for key, value in field.items() if key in {"/FT", "/V", "/DV", "/Ff"}}
                       for name, field in fields.items()},
            "pages": [page.indirect_reference.idnum for page in reader.pages], "labels": reader.page_labels,
            "text": [page.extract_text() for page in reader.pages],
            "destinations": {name: reader.get_destination_page_number(value) for name, value in reader.named_destinations.items()},
            "outline": [reader.get_destination_page_number(value) for value in reader.outline],
            "attachments": {name: [sha(value) for value in values] for name, values in reader.attachments.items()}}


def source_checks(source):
    result = inspect(source)
    require(set(result["fields"]) == {"group", "group.name", "group.keep", "code", "accept"}, "source field hierarchy changed")
    require(result["pages"] == [3, 4] and result["labels"] == ["A-1", "B-1"], "source page mapping changed")
    require(result["attachments"] == {"note.txt": [sha(ATTACHMENT)]}, "source attachment changed")
    _, objects = storage.inventory(source)
    for widget in [9, 10, 11]:
        require(objects[(widget, 0)].raw_get("/AP").idnum == 33 and objects[(33, 0)].raw_get("/N").idnum == 14, "source shared AP association changed")
    require(objects[(19, 0)]["/BaseFont"] == "/Courier" and objects[(21, 0)]["/BaseFont"] == "/Helvetica", "font collision control changed")
    require(list(objects[(19, 0)]["/Widths"]) == [600] * 224, "source explicit widths changed")
    return result


def verify(source, candidate):
    before, after = source_checks(source), inspect(candidate)
    old_reader, old = storage.inventory(source)
    reader, new = storage.inventory(candidate)
    require(set(old).issubset(new), "original objects removed")
    require(storage.trailer(old_reader) == storage.trailer(reader), "meaningful trailer changed")
    require(after["pages"] == [4, 3] and after["labels"] == ["B-1", "A-1"], "page mapping changed")
    require(after["text"] == [before["text"][index] for index in ORDER], "unrelated page content text changed")
    require(after["attachments"] == before["attachments"], "attachments changed")
    require(after["outline"] == [ORDER.index(index) for index in before["outline"]], "outlines changed")
    require(after["destinations"] == {name: ORDER.index(index) for name, index in before["destinations"].items()}, "named destinations changed")
    mutable = {(2, 0), (8, 0), (9, 0), (10, 0), (12, 0), (31, 0)}
    for identity, value in old.items():
        if identity not in mutable:
            require(storage.typed(new[identity]) == storage.typed(value), f"unrequested original object change: {identity}")
        if isinstance(value, StreamObject):
            require(isinstance(new[identity], StreamObject) and new[identity]._data == value._data, f"original encoded stream changed: {identity}")
    same_except(old[(2, 0)], new[(2, 0)], {"/Kids"})
    require([value.idnum for value in new[(2, 0)]["/Kids"]] == [4, 3], "page tree slots changed")
    same_except(old[(31, 0)], new[(31, 0)], {"/Nums"})
    nums = old[(31, 0)]["/Nums"]
    require(storage.typed(new[(31, 0)]["/Nums"]) == storage.typed(ArrayObject([NumberObject(0), nums[3], NumberObject(1), nums[1]])), "page label definitions changed")
    require(set(after["fields"]) == set(before["fields"]), "field registration changed")
    for name in set(before["fields"]) - set(REQUESTS):
        require(after["fields"][name] == before["fields"][name], f"unselected field changed: {name}")
    authorized, clones, effects = set(), set(), []
    for name, value in REQUESTS.items():
        spec = FIELDS[name]
        field = spec["field"]
        same_except(old[(field, 0)], new[(field, 0)], {"/V", "/AP"} if field == 12 else {"/V"})
        require(str(new[(field, 0)]["/V"]) == value and new[(field, 0)]["/V"].original_bytes == pdf_value(value), "requested field value/Unicode string encoding missing")
        for widget, page in spec["widgets"]:
            same_except(old[(widget, 0)], new[(widget, 0)], {"/V", "/AP"} if widget == field else {"/AP"})
            original_ap = old[(widget, 0)]["/AP"]
            actual_ap = new[(widget, 0)]["/AP"]
            original_association = old[(widget, 0)].raw_get("/AP")
            actual_association = new[(widget, 0)].raw_get("/AP")
            if isinstance(original_association, IndirectObject):
                require(isinstance(actual_association, IndirectObject), "indirect AP dictionary shape changed")
                identity = flatten.raw_identity(actual_association)
                require(identity not in old and identity not in authorized, "selected widget AP dictionary is not a private new clone")
                authorized.add(identity)
            else:
                require(isinstance(actual_association, DictionaryObject), "direct AP dictionary shape changed")
            same_except(original_ap, actual_ap, {"/N"})
            reference = flatten.raw_identity(actual_ap.raw_get("/N"))
            require(reference not in old and reference not in clones, "selected widget AP stream is not a private new clone")
            authorized.add(reference)
            clones.add(reference)
            original, actual = old[(spec["appearance"], 0)], new[reference]
            require(isinstance(actual, StreamObject), "cloned AP is not a stream")
            require(storage.stream_dictionary(actual) == storage.stream_dictionary(original), "new AP dictionary changed font/resources/geometry/private values")
            token_proof = verify_token_replacement(original.get_data(), actual.get_data(), spec["old"], value, reader)
            require(actual["/Resources"]["/Font"].raw_get("/F1").idnum == 19, "appearance font resolved through wrong page context")
            effects.append({"field": field, "widget": widget, "page": page, "source_appearance": spec["appearance"],
                            "output_appearance": list(reference), "value": value, "encoded_hex": value.encode("cp1252").hex(),
                            "font": 19, "cmap": 22, "font_size": 10, "original_advance": len(spec["old"])*6,
                            "replacement_advance": len(value)*6, "ink_bounds": [5.5, 7.5, 6+(len(value)-1)*6+6.5, 19],
                            "clip_bounds": [2, 2, 178 if spec["appearance"] == 14 else 148, 30], "token_replacement": token_proof})
    require(set(new)-set(old) == authorized, "unrequested new objects")
    return {"original_objects_retained": len(old), "new_objects": sorted([list(identity) for identity in authorized]),
            "effects": effects, "fields": after["fields"], "page_order": after["pages"], "labels": after["labels"],
            "original_encoded_streams": {str(identity): sha(value._data) for identity, value in old.items() if isinstance(value, StreamObject)}}


def clone_appearance(original, old_value, value):
    cloned = type(original)()
    cloned.update(original)
    cloned._data = zlib.compress(requested_program(original.get_data(), old_value, value))
    return cloned


def identity_expected(source):
    def requested(objects):
        next_number = 35
        owner = objects[(14, 0)].indirect_reference.pdf
        for name, value in REQUESTS.items():
            spec = FIELDS[name]
            objects[(spec["field"], 0)][NameObject("/V")] = ByteStringObject(pdf_value(value))
            for widget, _ in spec["widgets"]:
                original = objects[(spec["appearance"], 0)]
                reference = IndirectObject(next_number, 0, owner)
                objects[(next_number, 0)] = clone_appearance(original, spec["old"], value)
                next_number += 1
                widget_object = objects[(widget, 0)]
                association = widget_object.raw_get("/AP")
                ap = DictionaryObject(widget_object["/AP"])
                ap[NameObject("/N")] = reference
                if isinstance(association, IndirectObject):
                    widget_object[NameObject("/AP")] = IndirectObject(next_number, 0, owner)
                    objects[(next_number, 0)] = ap
                    next_number += 1
                else:
                    widget_object[NameObject("/AP")] = ap
        objects[(2, 0)][NameObject("/Kids")] = ArrayObject([objects[(4, 0)].indirect_reference, objects[(3, 0)].indirect_reference])
        nums = objects[(31, 0)]["/Nums"]
        objects[(31, 0)][NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[3], NumberObject(1), nums[1]])
    return geometry.rewrite_objects(source, requested)


def independent_expected_pdf(source):
    writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(source), strict=True))
    seen = set()
    def visit(reference, prefix=""):
        field = reference.get_object()
        name = ".".join(part for part in [prefix, str(field.get("/T", ""))] if part)
        if name in REQUESTS:
            value = REQUESTS[name]
            field[NameObject("/V")] = ByteStringObject(pdf_value(value))
            widgets = [field] if field.get("/Subtype") == "/Widget" else [child.get_object() for child in field["/Kids"]]
            for widget in widgets:
                ap = DictionaryObject(widget["/AP"])
                cloned = clone_appearance(ap["/N"], FIELDS[name]["old"], value)
                ap[NameObject("/N")] = writer._add_object(cloned)
                widget[NameObject("/AP")] = writer._add_object(ap)
            seen.add(name)
        else:
            for child in field.get("/Kids", []):
                visit(child, name)
    for reference in writer.root_object["/AcroForm"]["/Fields"]:
        visit(reference)
    require(seen == set(REQUESTS), "independent author missed fields")
    pages = list(writer.pages)
    root = writer.root_object.raw_get("/Pages")
    root.get_object()[NameObject("/Kids")] = ArrayObject([pages[index].indirect_reference for index in ORDER])
    root.get_object()[NameObject("/Count")] = NumberObject(2)
    for page in pages:
        page[NameObject("/Parent")] = root
    nums = writer.root_object["/PageLabels"]["/Nums"]
    writer.root_object["/PageLabels"][NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[3], NumberObject(1), nums[1]])
    writer.flattened_pages = [pages[index] for index in ORDER]
    output = io.BytesIO()
    writer.write(output)
    return output.getvalue()


def negative_controls(source, candidate):
    _, baseline_objects = storage.inventory(candidate)
    baseline = {identity: storage.typed(value) for identity, value in baseline_objects.items()}
    def lost_sibling(objects):
        objects[(7, 0)][NameObject("/Kids")] = ArrayObject([objects[(8, 0)].indirect_reference])
    def changed_original(objects):
        objects[(14, 0)]._data = zlib.compress(requested_program(objects[(14, 0)].get_data(), "Old", "Bad"))
    def wrong_value(objects):
        objects[(8, 0)][NameObject("/V")] = ByteStringObject(b"Wrong")
    def wrong_font(objects):
        appearance = objects[(9, 0)]["/AP"]["/N"]
        appearance["/Resources"]["/Font"][NameObject("/F1")] = objects[(21, 0)].indirect_reference
    def wrong_appearance(objects):
        appearance = objects[(9, 0)]["/AP"]["/N"]
        before = appearance.get_data()
        glyphs = REQUESTS["group.name"].encode("cp1252")
        tokens = [literal(glyphs), b"<" + glyphs.hex().upper().encode("ascii") + b">"]
        matches = [token for token in tokens if before.count(token + b" Tj") == 1]
        require(len(matches) == 1, "wrong-text control cannot identify one actual requested token")
        changed = before.replace(matches[0], b"(Wrong)", 1)
        require(changed != before, "wrong-text control did not change the actual appearance")
        appearance._data = zlib.compress(changed)
    def lost_style(objects):
        appearance = objects[(9, 0)]["/AP"]["/N"]
        appearance._data = zlib.compress(b"q BT /F1 10 Tf 1 0 0 1 6 10 Tm " + literal(REQUESTS["group.name"].encode("cp1252")) + b" Tj ET Q\n")
    def lost_link(objects):
        objects[(3, 0)][NameObject("/Annots")] = ArrayObject([value for value in objects[(3, 0)]["/Annots"] if value.idnum != 18])
    controls, negatives = {}, {}
    for name, mutate in [("lost_sibling", lost_sibling), ("changed_shared_original", changed_original), ("wrong_field_value", wrong_value),
                         ("wrong_font_context", wrong_font), ("wrong_appearance_text", wrong_appearance), ("lost_static_style", lost_style), ("lost_link", lost_link)]:
        corrupted = geometry.rewrite_objects(candidate, mutate)
        require(corrupted != candidate, f"negative {name} did not change its input")
        _, corrupted_objects = storage.inventory(corrupted)
        changed_objects = [list(identity) for identity, value in corrupted_objects.items()
                           if storage.typed(value) != baseline.get(identity)]
        require(changed_objects, f"negative {name} changed only serialization, not an object value")
        require(len(pypdf.PdfReader(io.BytesIO(corrupted), strict=True).pages) == 2, f"negative {name} cannot reparse")
        try:
            verify(source, corrupted)
        except RuntimeError as error:
            controls[name] = {"reparse": "passed", "detected": str(error), "sha256": sha(corrupted), "changed_objects": changed_objects}
        else:
            raise RuntimeError(f"oracle accepted {name}")
        negatives[name] = corrupted
    return controls, negatives


def render_checks(tool, source, paths, negatives, report):
    from PIL import Image, ImageChops, __version__ as pillow_version
    tool = tool.resolve(strict=True)
    work = REPO / "target/binas-validation/text-field"
    work.mkdir(parents=True, exist_ok=True)
    expected = independent_expected_pdf(source)
    expected_path = work / "independent-expected.pdf"
    expected_path.write_bytes(expected)
    inputs = {"expected": expected_path, "source": SOURCE, **paths}
    for name in ["changed_shared_original", "wrong_font_context", "wrong_appearance_text", "lost_static_style"]:
        path = work/f"negative-{name}.pdf"
        path.write_bytes(negatives[name])
        inputs[name] = path
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    images, records = {}, []
    for name, path in inputs.items():
        images[name] = []
        for page in [1, 2]:
            prefix = work/f"{name}-{page}"
            command = [str(tool), "-f", str(page), "-l", str(page), "-singlefile", "-r", "144", "-cropbox", "-png", str(path), str(prefix)]
            result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
            png = Path(str(prefix)+".png")
            with Image.open(png) as opened:
                image = opened.convert("RGB")
            images[name].append(image)
            records.append({"name": name, "page": page, "command": command, "stderr": result.stderr, "size": list(image.size),
                            "rgb_sha256": sha(image.tobytes()), "png_sha256": sha(png.read_bytes())})
    def same(left, right):
        return left.size == right.size and ImageChops.difference(left, right).getbbox() is None
    comparisons = {name: [same(left, right) for left, right in zip(images["expected"], images[name])] for name in inputs if name not in {"expected", "source"}}
    for name in paths:
        require(comparisons[name] == [True, True], f"{name} differs from independent all-page RGB144 expected")
    for name in ["changed_shared_original", "wrong_font_context", "wrong_appearance_text", "lost_static_style"]:
        require(comparisons[name] == [True, False], f"{name} did not change only alpha page")
    # The original unselected owner at [30,80,210,112] remains pixel-identical.
    crop = (60, 296, 420, 360)
    require(same(images["source"][0].crop(crop), images["expected"][1].crop(crop)), "independent author changed unselected shared owner")
    for name in paths:
        require(same(images["source"][0].crop(crop), images[name][1].crop(crop)), f"{name} changed unselected shared owner")
    report["rendering"] = {"result": "passed", "dpi": 144, "color": "RGB", "all_pages": True, "comparisons": comparisons,
                           "unselected_shared_owner_pixel_crop": list(crop), "unselected_shared_owner": "passed",
                           "expected_pdf_sha256": sha(expected), "expected_authorship": "independent pypdf source clone, AP clones and glyph-token replacement, explicit value encoding and reorder",
                           "pillow": pillow_version, "poppler": (version.stdout+version.stderr).strip(), "renderer_sha256": sha(tool.read_bytes()), "renders": records}


def generate_fixture():
    source = fixture_bytes()
    source_checks(source)
    require(not oracle.WARNINGS and not storage.WARNINGS, "source emitted reader warnings")
    SOURCE.write_bytes(source)
    PROVENANCE.write_text(json.dumps({"schema": 1, "license": "Apache-2.0", "origin": __doc__.strip(), "external_assets": [],
        "path": "fixtures/text-field-fill.pdf", "bytes": len(source), "sha256": sha(source), "generator": "text_field_oracle.py --generate-fixture",
        "generator_sha256": sha(Path(__file__).read_bytes()), "oracle_sha256": sha(Path(__file__).read_bytes()), "python": platform.python_version(), "pypdf": pypdf.__version__,
        "request": {"fields": REQUESTS, "page_order": ORDER}, "source_field_ids": FIELDS, "unselected_shared_owner": 11,
        "shared_ap_dictionary": 33, "appearance_font": 19, "page_font": 21, "cmap": 22, "explicit_widths": 600, "font_bbox": [-50,-250,650,900],
        "assurance": "Original synthetic source only; self-check and Binas outputs require separate gates."}, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    print(json.dumps({"source_sha256": sha(source), "bytes": len(source)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--generate-fixture", action="store_true")
    mode.add_argument("--structure-only", action="store_true")
    mode.add_argument("--pdftoppm", type=Path)
    args = parser.parse_args()
    require(pypdf.__version__ == "6.10.0", "install pinned pypdf6.10.0")
    if args.generate_fixture:
        generate_fixture()
        return
    source = SOURCE.read_bytes()
    provenance = json.loads(PROVENANCE.read_text(encoding="utf-8"))
    require(sha(source) == provenance["sha256"] and source == fixture_bytes(), "frozen text-field source/generator mismatch")
    require(sha(Path(__file__).read_bytes()) == provenance["oracle_sha256"], "text-field oracle changed; review its separate provenance hash")
    require(not oracle.WARNINGS and not storage.WARNINGS, "warnings before scoped gate")
    phases, outputs = [], []
    previous = flatten.checkpoint()
    source_checks(source)
    phases.append(flatten.warning_phase("source", previous))
    paths = {name: ROOT/f"evidence/text-field-{name}.pdf" for name in ["incremental", "compact"]}
    for name, path in paths.items():
        previous = flatten.checkpoint()
        candidate = path.read_bytes()
        if name == "incremental":
            require(candidate.startswith(source), "incremental prefix changed")
        outputs.append({"mode": name, "bytes": len(candidate), "sha256": sha(candidate), "checks": verify(source, candidate)})
        phases.append(flatten.warning_phase(name, previous, source, candidate))
    previous = flatten.checkpoint()
    controls, negatives = negative_controls(source, paths["compact"].read_bytes())
    phases.append(flatten.warning_phase("corruption_controls", previous))
    warning_controls = flatten.warning_gate_controls(source, paths["incremental"].read_bytes())
    report = {"schema": 1, "result": "passed", "scope": __doc__.strip(), "python": platform.python_version(), "pypdf": pypdf.__version__,
              "source_sha256": sha(source), "oracle_sha256": sha(Path(__file__).read_bytes()), "provenance_sha256": sha(PROVENANCE.read_bytes()),
              "example_sha256": sha((REPO/"crates/binas-pdf/examples/preservation_text_field.rs").read_bytes()),
              "helper_sha256": {name: sha((ROOT/name).read_bytes()) for name in ["flatten_oracle.py", "flate_oracle.py", "geometry_oracle.py", "oracle.py", "requirements.txt", "LICENSE"]},
              "request": {"fields": REQUESTS, "page_order": ORDER}, "outputs": outputs, "negative_controls": controls, "warning_gate_controls": warning_controls,
              "ignored_layout": ["xref offsets and representation", "trailer Size/Prev/XRefStm", "new AP stream encoding and object numbering"],
              "text_token_encodings": ["original escaped literal style", "canonical uppercase hex; exact same independently parsed glyph bytes"],
              "rendering": {"result": "not_checked"}}
    if args.pdftoppm:
        previous = flatten.checkpoint()
        render_checks(args.pdftoppm, source, paths, negatives, report)
        phases.append(flatten.warning_phase("independent_render_expectation", previous))
    require([warning for phase in phases for warning in phase["warnings"]] == oracle.WARNINGS == storage.WARNINGS, "reader warning escaped phase accounting")
    report["reader_warnings"] = phases
    name = "text-field-evidence.json" if args.pdftoppm else "text-field-structure.json"
    (ROOT/"evidence"/name).write_text(json.dumps(report, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "writer_outputs": 2, "corruption_controls": len(controls), "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
