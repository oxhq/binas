"""Original Apache-2.0 targeted form flattening fixture and independent gate.

Only group.answer is flattened, using two existing shared Form appearances with
a nonidentity Matrix, then pages reorder [1,0]. Unselected fields, checkbox and
links remain interactive. Detached objects remain: this is not secure erasure.
The gate checks all original objects/encoded streams and all-page RGB144 output;
it is a synthetic supported case, not general form or standards conformance.
"""
import argparse
import io
import json
import platform
import re
import subprocess
from pathlib import Path

import pypdf
from pypdf.generic import (ArrayObject, ContentStream, DecodedStreamObject,
                          DictionaryObject, IndirectObject,
                          NameObject, NumberObject, StreamObject)

import flate_oracle as storage
import geometry_oracle as geometry
import oracle

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
SOURCE = ROOT / "fixtures/targeted-flatten.pdf"
PROVENANCE = ROOT / "fixtures/targeted-flatten.provenance.json"
ORDER = [1, 0]
FIELD = "group.answer"
ATTACHMENT = b"Original targeted flatten attachment. Preserve NUL:\x00 and newline.\n"
SELECTED = {3: {"widget": 9, "content": 5, "rect": [30, 140, 210, 180]},
            4: {"widget": 10, "content": 6, "rect": [65, 110, 155, 190]}}
SPARSE_WARNING = "Xref table not zero-indexed. ID numbers for objects will be corrected."
sha, require = storage.sha, storage.require


def fixture_bytes():
    stream = storage.stream
    form = b"/Type /XObject /Subtype /Form /FormType 1 /Resources << /Font << /F1 19 0 R >> >>"
    cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /OriginalASCII def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfrange <20> <7E> <0020> endbfrange endcmap CMapName currentdict /CMap defineresource pop end end"
    def content(title):
        return (b"BT /F1 19 Tf 30 230 Td (" + title + b") Tj ET\n"
                b"q 1 0 0 1 230 205 cm /BinasFlatten0 Do Q\n"
                b"q 0.3 0.4 0.6 RG 1 w 20 20 260 185 re S Q\n"
                b"1 0 0 1 100 0 cm\n0 0 20 20 re W n\n")
    box = b"q 1 1 1 rg 0 0 24 24 re f 0.1 0.2 0.3 RG 1 w 1 1 22 22 re S Q\n"
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R /AcroForm 21 0 R /Metadata 22 0 R /Outlines 23 0 R /Names 25 0 R /PageLabels 29 0 R /Lang (en-US) /Private (original catalog) >>",
        2: b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 260] /Resources 30 0 R /Private (page tree) >>",
        3: b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [9 0 R 11 0 R 13 0 R] /Private (alpha page) >>",
        4: b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Annots [10 0 R 12 0 R] /Private (bravo page) >>",
        5: stream(content(b"PAGE ALPHA"), weak_flate=True),
        6: stream(content(b"PAGE BRAVO"), weak_flate=True),
        7: b"<< /T (group) /Kids [8 0 R 11 0 R] /Private (keep this field group) >>",
        8: b"<< /Parent 7 0 R /FT /Tx /Ff 0 /T (answer) /TU (Answer to flatten) /V (Shared answer) /DV (Original answer) /Kids [9 0 R 10 0 R] /Private (detached terminal retained) >>",
        9: b"<< /Type /Annot /Subtype /Widget /Parent 8 0 R /P 3 0 R /Rect [30 140 210 180] /F 4 /AP << /N 14 0 R >> /Private (detached alpha widget) >>",
        10: b"<< /Type /Annot /Subtype /Widget /Parent 8 0 R /P 4 0 R /Rect [65 110 155 190] /F 4 /AP << /N 14 0 R >> /Private (detached bravo widget) >>",
        11: b"<< /Type /Annot /Subtype /Widget /Parent 7 0 R /FT /Tx /Ff 0 /T (keep) /TU (Retained editable sibling) /V (Keep editing) /DV (Keep editing) /P 3 0 R /Rect [30 70 200 100] /F 4 /AP << /N 15 0 R >> /Private (editable sibling) >>",
        12: b"<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 0 /T (accept) /V /Yes /DV /Off /AS /Yes /P 4 0 R /Rect [220 65 244 89] /F 4 /AP << /N << /Off 16 0 R /Yes 17 0 R >> >> /Private (retained checkbox) >>",
        13: b"<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [30 35 200 55] /Dest [4 0 R /Fit] /Border [0 0 1] /Contents (Retained link) >>",
        14: stream(b"q 0.1 0.35 0.7 rg 0 0 80 20 re f 1 1 1 rg BT /F1 9 Tf 4 7 Td (Shared answer) Tj ET Q\n", form + b" /BBox [0 0 80 20] /Matrix [1 0.25 0.5 1 7 -3] /Private (shared encoded selected appearance)", True),
        15: stream(b"q 0.85 0.95 0.85 rg 0 0 170 30 re f 0 0 0 rg BT /F1 12 Tf 5 10 Td (Keep editing) Tj ET Q\n", form + b" /BBox [0 0 170 30]"),
        16: stream(box, form + b" /BBox [0 0 24 24]"),
        17: stream(box + b"q 0.05 0.45 0.2 RG 3 w 5 12 m 10 7 l 19 18 l S Q\n", form + b" /BBox [0 0 24 24]"),
        18: stream(b"q 0.8 0.25 0.1 rg 0 0 30 20 re f Q\n", form + b" /BBox [0 0 30 20] /Private (existing colliding XObject name)"),
        19: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 20 0 R >>",
        20: stream(cmap),
        21: b"<< /Fields [7 0 R 12 0 R] /NeedAppearances false /DA (/F1 12 Tf 0 g) /DR << /Font << /F1 19 0 R >> >> /Private (preserved AcroForm) >>",
        22: stream(b'<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Original targeted flatten fixture</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>', b"/Type /Metadata /Subtype /XML"),
        23: b"<< /Type /Outlines /First 24 0 R /Last 24 0 R /Count 1 >>",
        24: b"<< /Title (Bravo page) /Parent 23 0 R /Dest [4 0 R /Fit] >>",
        25: b"<< /Dests << /Names [(bravo) [4 0 R /Fit]] >> /EmbeddedFiles << /Names [(note.txt) 26 0 R] >> >>",
        26: b"<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F 27 0 R >> /Desc (Exact original attachment) >>",
        27: stream(ATTACHMENT, b"/Type /EmbeddedFile /Subtype /text#2Fplain"),
        28: b"<< /Title (Original targeted flatten fixture) /Author (Binas original Apache-2.0 QA) >>",
        29: b"<< /Nums [0 << /S /D /P (A-) /St 1 /Private (alpha label) >> 1 << /S /D /P (B-) /St 1 /Private (bravo label) >>] /Private (label tree) >>",
        30: b"<< /Font << /F1 19 0 R >> /XObject << /BinasFlatten0 18 0 R >> /ProcSet [/PDF /Text] /Private (shared inherited resources) >>",
    }
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for number, value in sorted(objects.items()):
        offsets[number] = len(output)
        output += f"{number} 0 obj\n".encode() + value + b"\nendobj\n"
    xref = len(output)
    output += b"xref\n0 31\n0000000000 65535 f \n"
    for number in range(1, 31):
        output += f"{offsets[number]:010} 00000 n \n".encode()
    output += ("trailer\n<< /Size 31 /Root 1 0 R /Info 28 0 R "
               "/ID [<5441524745544544464C415454454E3031> <5441524745544544464C415454454E3031>] >>\n"
               f"startxref\n{xref}\n%%EOF\n").encode()
    return bytes(output)


def placement(appearance, rect):
    a, b, c, d, e, f = map(float, appearance.get("/Matrix", [1, 0, 0, 1, 0, 0]))
    x0, y0, x1, y1 = map(float, appearance["/BBox"])
    corners = [(a*x + c*y + e, b*x + d*y + f) for x in [x0, x1] for y in [y0, y1]]
    low_x, high_x = min(p[0] for p in corners), max(p[0] for p in corners)
    low_y, high_y = min(p[1] for p in corners), max(p[1] for p in corners)
    sx, sy = (rect[2] - rect[0]) / (high_x - low_x), (rect[3] - rect[1]) / (high_y - low_y)
    return [sx, 0, 0, sy, rect[0] - sx*low_x, rect[1] - sy*low_y]


def operations(data, reader):
    stream = DecodedStreamObject()
    stream.set_data(data)
    return ContentStream(stream, reader).operations


def raw_identity(value):
    require(isinstance(value, IndirectObject), "expected an indirect object identity")
    return value.idnum, value.generation


def inspect(data):
    reader, objects = storage.inventory(data)
    fields = reader.get_fields()
    require(fields is not None, "missing field registration")
    return {"reader": reader, "objects": objects,
            "fields": {name: {key: storage.typed(value) for key, value in field.items() if key in {"/FT", "/V", "/DV"}}
                       for name, field in fields.items()},
            "pages": [page.indirect_reference.idnum for page in reader.pages],
            "labels": reader.page_labels,
            "text": [page.extract_text() for page in reader.pages],
            "destinations": {name: reader.get_destination_page_number(value) for name, value in reader.named_destinations.items()},
            "outline": [reader.get_destination_page_number(value) for value in reader.outline],
            "attachments": {name: [sha(value) for value in values] for name, values in reader.attachments.items()}}


def source_checks(source):
    result = inspect(source)
    require(result["pages"] == [3, 4] and result["labels"] == ["A-1", "B-1"], "source pages/labels changed")
    require(set(result["fields"]) == {"group", FIELD, "group.keep", "accept"}, "source field hierarchy changed")
    require(result["attachments"] == {"note.txt": [sha(ATTACHMENT)]}, "source attachment changed")
    for page, expected in zip([3, 4], [[2, 0, 0, 1, 16, 143], [1, 0, 0, 2, 58, 116]]):
        require(placement(result["objects"][(14, 0)], SELECTED[page]["rect"]) == expected, "source appearance placement changed")
    return result


def verify(source, candidate):
    before, after = source_checks(source), inspect(candidate)
    # pypdf page flattening materializes inherited leaf keys; use separate,
    # unwalked readers for the raw object-preservation comparison.
    _, old = storage.inventory(source)
    reader, new = storage.inventory(candidate)
    require(set(old).issubset(new), "flattening removed original detached objects")
    require(storage.trailer(before["reader"]) == storage.trailer(reader), "meaningful trailer changed")
    require(after["pages"] == [4, 3] and after["labels"] == ["B-1", "A-1"], "requested page/label mapping missing")
    require(after["fields"] == {name: value for name, value in before["fields"].items() if name != FIELD}, "selected field remains active or an unrelated field changed")
    require(after["attachments"] == before["attachments"], "attachments changed")
    require(after["destinations"] == {name: ORDER.index(index) for name, index in before["destinations"].items()}, "named destinations changed")
    require(after["outline"] == [ORDER.index(index) for index in before["outline"]], "outlines changed")
    require(after["text"] == [before["text"][index] + "Shared answer" for index in ORDER],
            "original page text or newly flattened appearance text changed")
    mutable = {(2, 0), (3, 0), (4, 0), (7, 0), (29, 0)}
    for identity, value in old.items():
        if identity not in mutable:
            require(storage.typed(new[identity]) == storage.typed(value), f"unrequested original object change: {identity}")
        if isinstance(value, StreamObject):
            require(isinstance(new[identity], StreamObject) and new[identity]._data == value._data,
                    f"original encoded stream changed: {identity}")
    def same_except(source_object, candidate_object, keys, additions=()):
        require(set(candidate_object) - set(source_object) <= set(additions), "unrequested dictionary keys added")
        require(set(source_object) - set(candidate_object) <= set(keys), "unrequested dictionary keys removed")
        for key, value in source_object.items():
            if key not in keys:
                require(storage.typed(candidate_object.raw_get(key)) == storage.typed(value), f"unrequested dictionary key change: {key}")
    same_except(old[(2, 0)], new[(2, 0)], {"/Kids"})
    require([raw_identity(value) for value in new[(2, 0)]["/Kids"]] == [(4, 0), (3, 0)], "page tree changed beyond reorder")
    same_except(old[(7, 0)], new[(7, 0)], {"/Kids"})
    require([raw_identity(value) for value in new[(7, 0)]["/Kids"]] == [(11, 0)], "nested sibling registration lost")
    same_except(old[(29, 0)], new[(29, 0)], {"/Nums"})
    nums = old[(29, 0)]["/Nums"]
    expected_nums = ArrayObject([NumberObject(0), nums[3], NumberObject(1), nums[1]])
    require(storage.typed(new[(29, 0)].raw_get("/Nums")) == storage.typed(expected_nums), "page labels changed beyond requested mapping")
    authorized_new, effects = set(), []
    for page, selected in SELECTED.items():
        original, current = old[(page, 0)], new[(page, 0)]
        same_except(original, current, {"/Contents", "/Annots", "/Resources"}, {"/Resources"})
        expected_annots = [raw_identity(value) for value in original["/Annots"] if value.idnum != selected["widget"]]
        require([raw_identity(value) for value in current["/Annots"]] == expected_annots, "selected widget retained or unrelated annotation/link lost")
        original_resources, resources = old[(30, 0)], current["/Resources"]
        same_except(original_resources, resources, {"/XObject"})
        xobjects = resources["/XObject"]
        require(storage.typed(xobjects.raw_get("/BinasFlatten0")) == storage.typed(original_resources["/XObject"].raw_get("/BinasFlatten0")), "existing XObject resource collision")
        added_names = set(xobjects) - set(original_resources["/XObject"])
        require(len(added_names) == 1 and set(xobjects) == set(original_resources["/XObject"]) | added_names, "unexpected page XObject inventory")
        name = added_names.pop()
        require(raw_identity(xobjects.raw_get(name)) == (14, 0), "flatten painting does not use original selected appearance")
        for value in [current.raw_get("/Resources"), resources.raw_get("/XObject")]:
            if isinstance(value, IndirectObject):
                identity = raw_identity(value)
                require(identity not in old, "flatten mutated or reused a shared source dictionary")
                authorized_new.add(identity)
        contents = current.raw_get("/Contents")
        require(isinstance(contents, ArrayObject), "flatten did not retain original content in an isolated array")
        refs = [raw_identity(value) for value in contents]
        original_ref = (selected["content"], 0)
        require(refs.count(original_ref) == 1, "original page stream missing or duplicated")
        position = refs.index(original_ref)
        require(position > 0 and position < len(refs)-1, "original page content has no save/restore wrapper")
        for identity in refs[:position] + refs[position+1:]:
            require(identity not in old and isinstance(new[identity], StreamObject), "wrapper uses an unrelated original object")
            require(set(new[identity]) <= {"/Length", "/Filter", "/DecodeParms"}, "unexpected wrapper stream semantics")
            authorized_new.add(identity)
        prefix = b"\n".join(new[identity].get_data() for identity in refs[:position])
        suffix = b"\n".join(new[identity].get_data() for identity in refs[position+1:])
        require(operations(prefix, reader) == [([], b"q")], "original graphics state is not isolated")
        expected_matrix = placement(old[(14, 0)], selected["rect"])
        suffix_ops = operations(suffix, reader)
        require([op for _, op in suffix_ops] == [b"n", b"Q", b"q", b"cm", b"Do", b"Q"], "flatten paint operators do not restore default graphics state")
        require(all(not args for args, op in suffix_ops if op in {b"n", b"q", b"Q"}), "unexpected graphics-state operands")
        require([float(value) for value in suffix_ops[3][0]] == expected_matrix, "wrong transformed-BBox to widget-Rect placement")
        require(suffix_ops[4][0] == [name], "flatten paints wrong XObject resource")
        effects.append({"field": 8, "widget": selected["widget"], "page": page, "output_page_index": after["pages"].index(page),
                        "appearance": 14, "resource_name": str(name), "placement_matrix": expected_matrix,
                        "original_content": selected["content"], "new_content_streams": [list(identity) for identity in refs if identity != original_ref]})
    require(set(new) - set(old) == authorized_new, "unrequested new objects outside flatten wrappers/resources")
    return {"source_objects_retained": len(old), "new_objects": sorted([list(identity) for identity in authorized_new]),
            "effects": effects, "fields": after["fields"], "page_order": after["pages"], "labels": after["labels"],
            "text": after["text"], "attachments": after["attachments"],
            "original_encoded_streams": {str(identity): sha(value._data) for identity, value in old.items() if isinstance(value, StreamObject)}}


def independent_expected_pdf(source):
    writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(source), strict=True))
    pages = list(writer.pages)
    acroform = writer.root_object["/AcroForm"]
    group = acroform["/Fields"][0].get_object()
    selected = next(reference.get_object() for reference in group["/Kids"] if reference.get_object().get("/T") == "answer")
    widgets = [reference.get_object() for reference in selected["/Kids"]]
    for page, widget in zip(pages, widgets):
        page[NameObject("/Annots")] = ArrayObject([reference for reference in page["/Annots"] if reference != widget.indirect_reference])
        resources = DictionaryObject(page["/Resources"])
        xobjects = DictionaryObject(resources["/XObject"])
        name = NameObject("/IndependentFlatten")
        require(name not in xobjects, "independent expected name collides")
        xobjects[name] = widget["/AP"].raw_get("/N")
        resources[NameObject("/XObject")] = xobjects
        page[NameObject("/Resources")] = resources
        before, after = DecodedStreamObject(), DecodedStreamObject()
        before.set_data(b"q\n")
        matrix = placement(widget["/AP"]["/N"], list(map(float, widget["/Rect"])))
        after.set_data(("\nn\nQ\nq\n" + " ".join(format(value, ".12g") for value in matrix) + " cm\n/IndependentFlatten Do\nQ\n").encode("ascii"))
        page[NameObject("/Contents")] = ArrayObject([writer._add_object(before), page.raw_get("/Contents"), writer._add_object(after)])
    group[NameObject("/Kids")] = ArrayObject([reference for reference in group["/Kids"] if reference.get_object().get("/T") != "answer"])
    root_pages = writer.root_object.raw_get("/Pages")
    root_pages.get_object()[NameObject("/Kids")] = ArrayObject([pages[index].indirect_reference for index in ORDER])
    root_pages.get_object()[NameObject("/Count")] = NumberObject(2)
    for page in pages:
        page[NameObject("/Parent")] = root_pages
    labels = writer.root_object["/PageLabels"]
    nums = labels["/Nums"]
    labels[NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[3], NumberObject(1), nums[1]])
    writer.flattened_pages = [pages[index] for index in ORDER]
    output = io.BytesIO()
    writer.write(output)
    expected = output.getvalue()
    check = inspect(expected)
    require(set(check["fields"]) == {"group", "group.keep", "accept"}, "independent author lost unselected fields")
    require(check["labels"] == ["B-1", "A-1"], "independent author lost page labels")
    return expected


def identity_preserving_expected(source):
    """Exercise exact-object gate with independent raw-object author, not engine bytes."""
    def requested(objects):
        objects[(2, 0)][NameObject("/Kids")] = ArrayObject([objects[(4, 0)].indirect_reference, objects[(3, 0)].indirect_reference])
        objects[(7, 0)][NameObject("/Kids")] = ArrayObject([objects[(11, 0)].indirect_reference])
        nums = objects[(29, 0)]["/Nums"]
        objects[(29, 0)][NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[3], NumberObject(1), nums[1]])
        for index, (page_id, selected) in enumerate(SELECTED.items()):
            page = objects[(page_id, 0)]
            page[NameObject("/Annots")] = ArrayObject([value for value in page["/Annots"] if value.idnum != selected["widget"]])
            resources = DictionaryObject(objects[(30, 0)])
            xobjects = DictionaryObject(resources["/XObject"])
            name = NameObject("/BinasFlatten1")
            xobjects[name] = objects[(14, 0)].indirect_reference
            resources[NameObject("/XObject")] = xobjects
            page[NameObject("/Resources")] = resources
            prefix, suffix = DecodedStreamObject(), DecodedStreamObject()
            prefix.set_data(b"q\n")
            matrix = placement(objects[(14, 0)], selected["rect"])
            suffix.set_data(("n\nQ\nq\n" + " ".join(format(value, ".12g") for value in matrix) + " cm\n/BinasFlatten1 Do\nQ\n").encode("ascii"))
            first = 31 + 2*index
            objects[(first, 0)], objects[(first+1, 0)] = prefix, suffix
            owner = objects[(14, 0)].indirect_reference.pdf
            page[NameObject("/Contents")] = ArrayObject([IndirectObject(first, 0, owner), objects[(selected["content"], 0)].indirect_reference, IndirectObject(first+1, 0, owner)])
    return geometry.rewrite_objects(source, requested)


def negative_controls(source, candidate):
    def lost_sibling(objects):
        objects[(7, 0)][NameObject("/Kids")] = ArrayObject([])
    def lost_link(objects):
        objects[(3, 0)][NameObject("/Annots")] = ArrayObject([value for value in objects[(3, 0)]["/Annots"] if value.idnum != 13])
    def resource_collision(objects):
        objects[(3, 0)]["/Resources"]["/XObject"][NameObject("/BinasFlatten0")] = objects[(14, 0)].indirect_reference
    def selected_registered(objects):
        objects[(7, 0)]["/Kids"].append(objects[(8, 0)].indirect_reference)
    def changed_appearance(objects):
        appearance = objects[(14, 0)]
        # Flate data is intentionally replaced through an independent stream encoder.
        import zlib
        appearance._data = zlib.compress(b"q 1 0 0 rg 0 0 80 20 re f Q\n")
    def wrong_placement(objects):
        suffix = objects[(3, 0)]["/Contents"][-1].get_object()
        original = suffix.get_data()
        changed, count = re.subn(rb"2(?:\.0+)? 0(?:\.0+)? 0(?:\.0+)? 1(?:\.0+)? 16(?:\.0+)? 143(?:\.0+)? cm",
                                b"2 0 0 1 36 143 cm", original)
        require(count == 1, "wrong-placement control did not mutate exactly one matrix")
        suffix.set_data(changed)
    results, negatives = {}, {}
    for name, mutate in [("lost_sibling", lost_sibling), ("lost_link", lost_link),
                         ("resource_collision", resource_collision), ("selected_still_registered", selected_registered),
                         ("changed_appearance", changed_appearance), ("wrong_placement", wrong_placement)]:
        corrupted = geometry.rewrite_objects(candidate, mutate)
        require(len(pypdf.PdfReader(io.BytesIO(corrupted), strict=True).pages) == 2, f"negative {name} cannot reparse")
        try:
            verify(source, corrupted)
        except RuntimeError as error:
            results[name] = {"reparse": "passed", "detected": str(error), "sha256": sha(corrupted)}
        else:
            raise RuntimeError(f"oracle accepted {name} corruption")
        negatives[name] = corrupted
    return results, negatives


def render_checks(tool, source, paths, negatives, report):
    from PIL import Image, ImageChops, __version__ as pillow_version
    tool = tool.resolve(strict=True)
    work = REPO / "target/binas-validation/flatten"
    work.mkdir(parents=True, exist_ok=True)
    expected = independent_expected_pdf(source)
    expected_path = work / "independent-expected.pdf"
    expected_path.write_bytes(expected)
    inputs = {"expected": expected_path, "source": SOURCE, **paths}
    for name in ["wrong_placement", "changed_appearance", "resource_collision"]:
        path = work / f"negative-{name}.pdf"
        path.write_bytes(negatives[name])
        inputs[name] = path
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    images, records = {}, []
    for name, path in inputs.items():
        images[name] = []
        for page in [1, 2]:
            prefix = work / f"{name}-{page}"
            command = [str(tool), "-f", str(page), "-l", str(page), "-singlefile", "-r", "144", "-cropbox", "-png", str(path), str(prefix)]
            result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
            png = Path(str(prefix) + ".png")
            with Image.open(png) as opened:
                image = opened.convert("RGB")
            images[name].append(image)
            records.append({"name": name, "page": page, "command": command, "stderr": result.stderr,
                            "size": list(image.size), "rgb_sha256": sha(image.tobytes()), "png_sha256": sha(png.read_bytes())})
    def same(left, right):
        return left.size == right.size and ImageChops.difference(left, right).getbbox() is None
    comparisons = {name: [same(left, right) for left, right in zip(images["expected"], images[name])]
                   for name in inputs if name not in {"expected", "source"}}
    comparisons["source_requested_page_mapping"] = [same(left, images["source"][index]) for left, index in zip(images["expected"], ORDER)]
    require(comparisons["source_requested_page_mapping"] == [True, True], "independent expected flatten changed original page appearance")
    for name in paths:
        require(comparisons[name] == [True, True], f"{name} differs from independent all-page RGB144 expected rendering")
    for name, expected_match in [("wrong_placement", [True, False]), ("resource_collision", [True, False]), ("changed_appearance", [False, False])]:
        require(comparisons[name] == expected_match, f"{name} did not alter exactly the expected rendered pages")
    report["rendering"] = {"result": "passed", "color": "RGB", "dpi": 144, "all_pages": True,
                           "expected_pdf_sha256": sha(expected), "expected_authorship": "pypdf source clone, selected registry/Annots removal, transformed BBox placement, graphics-state isolation and reorder",
                           "pillow": pillow_version, "poppler": (version.stdout + version.stderr).strip(),
                           "renderer_sha256": sha(tool.read_bytes()), "comparisons": comparisons, "renders": records}


def checkpoint():
    return len(oracle.WARNINGS), len(storage.WARNINGS)


def warning_phase(name, previous, source=None, candidate=None):
    warnings = oracle.WARNINGS[previous[0]:]
    require(warnings == storage.WARNINGS[previous[1]:], "reader warning captures disagree")
    return assess_warnings(name, warnings, source, candidate)


def assess_warnings(name, warnings, source=None, candidate=None):
    result = {"phase": name, "warnings": list(warnings), "result": "passed"}
    if warnings:
        require(name == "incremental" and all(value == SPARSE_WARNING for value in warnings), f"unacknowledged reader warnings in {name}: {warnings}")
        result["xref_proof"] = sparse_xref_proof(source, candidate)
        result["result"] = "acknowledged_with_raw_xref_proof"
    return result


def sparse_xref_proof(source, candidate):
    require(candidate.startswith(source), "xref proof: exact original prefix lost")
    def last_xref(data):
        match = re.search(rb"startxref\s+(\d+)\s+%%EOF\s*\Z", data)
        require(match is not None, "xref proof: missing terminal startxref")
        value = int(match[1])
        require(value < match.start() and data[value:value+5] == b"xref\n", "xref proof: invalid classic xref offset")
        return value
    previous, current = last_xref(source), last_xref(candidate)
    require(current >= len(source), "xref proof: latest section not appended")
    stream = io.BytesIO(candidate)
    stream.seek(current + 5)
    entries = []
    while True:
        line = stream.readline()
        if line == b"trailer\n":
            break
        section = re.fullmatch(rb"(\d+) (\d+)\n", line)
        require(section is not None, "xref proof: malformed subsection")
        first, count = map(int, section.groups())
        require(0 < first and 0 < count <= len(candidate)//20 and (not entries or first > entries[-1]["object"]), "xref proof: invalid subsection range")
        for number in range(first, first+count):
            entry_position = stream.tell()
            entry = re.fullmatch(rb"(\d{10}) (\d{5}) n \n", stream.readline())
            require(entry is not None, "xref proof: malformed entry")
            offset, generation = map(int, entry.groups())
            header = f"{number} {generation} obj\n".encode()
            require(len(source) <= offset < current and candidate[offset:offset+len(header)] == header, "xref proof: wrong object offset/header")
            entries.append({"object": number, "generation": generation, "offset": offset, "xref_entry_offset": entry_position})
    require(entries, "xref proof: empty latest section")
    tail = stream.read()
    trailer, separator, ending = tail.partition(b"\nstartxref\n")
    require(separator and trailer.startswith(b"<<") and trailer.endswith(b">>") and re.fullmatch(str(current).encode()+rb"\n%%EOF\s*", ending), "xref proof: malformed trailer/tail")
    require(re.findall(rb"/Prev\s+(\d+)(?=\s|/|>>)", trailer) == [str(previous).encode()], "xref proof: wrong previous revision")
    # Pinned pypdf 6.10.0 warns whenever a classic section's first subsection is
    # above zero. Strict mode does not perform its optional xref_index correction.
    return {"result": "passed", "source_xref": previous, "latest_xref": current, "entries": entries,
            "reader_source": {"version": pypdf.__version__, "path": "pypdf/_reader.py", "sha256": sha(Path(pypdf._reader.__file__).read_bytes()),
                              "warning": "_read_standard_xref_table: first_time and num != 0 and self.strict",
                              "correction": "read: self.xref_index and not self.strict"}}


def warning_gate_controls(source, candidate):
    proof = sparse_xref_proof(source, candidate)
    first = proof["entries"][0]
    entry = first["xref_entry_offset"]
    bad_offset = candidate[:entry] + f"{first['offset']+1:010}".encode() + candidate[entry+10:]
    header = first["offset"]
    bad_header = candidate[:header] + b"9" + candidate[header+1:]
    latest, previous = proof["latest_xref"], proof["source_xref"]
    needle = f"/Prev {previous}".encode()
    require(candidate[latest:].count(needle) == 1, "ambiguous Prev warning control")
    bad_previous = candidate[:latest] + candidate[latest:].replace(needle, f"/Prev {previous+1}".encode(), 1)
    controls = {}
    for name, data, warnings, phase in [
            ("bad_xref_offset", bad_offset, [SPARSE_WARNING], "incremental"),
            ("mismatched_object_header", bad_header, [SPARSE_WARNING], "incremental"),
            ("wrong_previous_revision", bad_previous, [SPARSE_WARNING], "incremental"),
            ("unknown_reader_warning", candidate, [SPARSE_WARNING, "Injected unknown reader warning"], "incremental"),
            ("sparse_warning_on_compact", candidate, [SPARSE_WARNING], "compact")]:
        try:
            assess_warnings(phase, warnings, source, data)
        except RuntimeError as error:
            controls[name] = {"detected": str(error), "sha256": sha(data), "gate": "assess_warnings",
                              "input_warnings": warnings, "phase": phase}
        else:
            raise RuntimeError(f"warning gate accepted {name}")
    return controls


def generate_fixture():
    source = fixture_bytes()
    source_checks(source)
    require(not oracle.WARNINGS and not storage.WARNINGS, "source emits reader warnings")
    SOURCE.write_bytes(source)
    PROVENANCE.write_text(json.dumps({"schema": 1, "license": "Apache-2.0", "origin": __doc__.strip(), "external_assets": [],
        "path": "fixtures/targeted-flatten.pdf", "bytes": len(source), "sha256": sha(source),
        "generator": "flatten_oracle.py --generate-fixture", "generator_sha256": sha(Path(__file__).read_bytes()), "oracle_sha256": sha(Path(__file__).read_bytes()),
        "python": platform.python_version(), "pypdf": pypdf.__version__, "page_objects": [3, 4], "selected_field": 8,
        "selected_widgets": [9, 10], "shared_appearance": 14, "retained_fields": [11, 12], "retained_link": 13,
        "inherited_resources": 30, "colliding_resource_name": "BinasFlatten0", "request": {"field": FIELD, "page_order": ORDER},
        "assurance": "Source strict-reader check only. Independent expected/self-check and actual engine outputs are separate gates."}, indent=2)+"\n", encoding="utf-8")
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
    require(sha(source) == provenance["sha256"] and source == fixture_bytes(), "frozen flatten source differs from provenance/generator")
    require(sha(Path(__file__).read_bytes()) == provenance["oracle_sha256"], "flatten oracle changed; review separate provenance hash")
    require(not oracle.WARNINGS and not storage.WARNINGS, "warnings before scoped gate")
    phases = []
    before = checkpoint()
    source_checks(source)
    phases.append(warning_phase("source", before))
    paths = {name: ROOT/f"evidence/flatten-{name}.pdf" for name in ["incremental", "compact"]}
    outputs = []
    for name, path in paths.items():
        before = checkpoint()
        candidate = path.read_bytes()
        if name == "incremental":
            require(candidate.startswith(source), "incremental output lost exact source prefix")
        outputs.append({"mode": name, "bytes": len(candidate), "sha256": sha(candidate), "checks": verify(source, candidate)})
        phases.append(warning_phase(name, before, source, candidate))
    before = checkpoint()
    controls, negatives = negative_controls(source, paths["compact"].read_bytes())
    phases.append(warning_phase("corruption_controls", before))
    warning_controls = warning_gate_controls(source, paths["incremental"].read_bytes())
    report = {"schema": 1, "result": "passed", "scope": __doc__.strip(), "python": platform.python_version(), "pypdf": pypdf.__version__,
              "source_sha256": sha(source), "oracle_sha256": sha(Path(__file__).read_bytes()), "provenance_sha256": sha(PROVENANCE.read_bytes()),
              "example_sha256": sha((REPO/"crates/binas-pdf/examples/preservation_flatten.rs").read_bytes()),
              "helper_sha256": {name: sha((ROOT/name).read_bytes()) for name in ["flate_oracle.py", "geometry_oracle.py", "oracle.py", "requirements.txt", "LICENSE"]},
              "request": {"field": FIELD, "page_order": ORDER}, "outputs": outputs, "negative_controls": controls,
              "warning_gate_controls": warning_controls,
              "ignored_layout": ["xref offsets and representation", "trailer Size/Prev/XRefStm", "wrapper resource names and object numbering"],
              "rendering": {"result": "not_checked"}}
    if args.pdftoppm:
        before = checkpoint()
        render_checks(args.pdftoppm, source, paths, negatives, report)
        phases.append(warning_phase("independent_render_expectation", before))
    recorded = [warning for phase in phases for warning in phase["warnings"]]
    require(recorded == oracle.WARNINGS == storage.WARNINGS, "reader warning escaped phase accounting")
    report["reader_warnings"] = phases
    receipt = "flatten-evidence.json" if args.pdftoppm else "flatten-structure.json"
    (ROOT/"evidence"/receipt).write_text(json.dumps(report, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "writer_outputs": 2, "corruption_controls": len(controls), "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
