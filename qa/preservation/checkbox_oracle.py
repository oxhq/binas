"""Original Apache-2.0 retained checkbox/reorder fixture and independent gate.

Two checkbox states change using their existing shared normal appearances, then
pages reorder [1,0]. This synthetic case checks field/widget registration, exact
unrelated PDF objects and all-page RGB144 rendering; it is not full form support,
accessibility assurance, appearance generation or PDF standards conformance.
"""
import argparse
import io
import json
import platform
import re
import subprocess
from pathlib import Path

import pypdf
from pypdf.generic import ArrayObject, NameObject, NumberObject, StreamObject

import flate_oracle as storage
import geometry_oracle as geometry
import oracle

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
SOURCE = ROOT / "fixtures/checkbox-states.pdf"
PROVENANCE = ROOT / "fixtures/checkbox-states.provenance.json"
ORDER = [1, 0]
REQUESTS = {"accept": "/Yes", "preferences.subscribe": "/Off"}
ATTACHMENT = b"Original checkbox attachment; preserve NUL:\x00 and newline.\n"
sha = storage.sha
require = storage.require
SPARSE_XREF_WARNING = "Xref table not zero-indexed. ID numbers for objects will be corrected."


def warning_checkpoint():
    return len(oracle.WARNINGS), len(storage.WARNINGS)


def warnings_since(checkpoint):
    primary = oracle.WARNINGS[checkpoint[0]:]
    secondary = storage.WARNINGS[checkpoint[1]:]
    require(primary == secondary, "independent reader warning captures disagree")
    return list(primary)


def sparse_incremental_xref_proof(source, candidate):
    """Check this fixture's sparse append independently of pypdf's xref parser.

    Pinned pypdf 6.10.0 `_read_standard_xref_table` warns for every first
    subsection above zero, including a /Prev-linked incremental section. Its
    `read` method only attempts reindexing when strict is false. We retain
    strict=True and acknowledge that exact warning only after checking raw
    offsets, object identities and the unchanged original revision below.
    This bounded fixture check is not a general PDF conformance claim.
    """
    require(candidate.startswith(source), "xref proof: original revision prefix changed")
    def startxref(data):
        match = re.search(rb"startxref\s+(\d+)\s+%%EOF\s*\Z", data)
        require(match is not None, "xref proof: missing terminal startxref")
        offset = int(match[1])
        require(offset < match.start() and data[offset:offset + 5] == b"xref\n",
                "xref proof: startxref does not address a classic xref section")
        return offset
    previous, current = startxref(source), startxref(candidate)
    require(len(source) <= current, "xref proof: latest xref is not appended")
    stream = io.BytesIO(candidate)
    stream.seek(current + 5)
    entries, subsection_starts = [], []
    while True:
        line = stream.readline()
        if line == b"trailer\n":
            break
        subsection = re.fullmatch(rb"(\d+) (\d+)\n", line)
        require(subsection is not None, "xref proof: malformed subsection header")
        first, count = map(int, subsection.groups())
        require(0 < count <= 25 and 0 < first < 26 and first + count <= 26,
                "xref proof: subsection outside frozen object inventory")
        require(not entries or first > entries[-1]["object"], "xref proof: overlapping or unordered subsections")
        subsection_starts.append(first)
        for number in range(first, first + count):
            entry_position = stream.tell()
            entry = re.fullmatch(rb"(\d{10}) (\d{5}) n \n", stream.readline())
            require(entry is not None, "xref proof: unsupported or malformed xref entry")
            offset, generation = map(int, entry.groups())
            require(len(source) <= offset < current, "xref proof: object offset outside appended revision")
            expected_header = f"{number} {generation} obj\n".encode("ascii")
            require(candidate[offset:offset + len(expected_header)] == expected_header,
                    "xref proof: offset does not match exact object/generation header")
            entries.append({"object": number, "generation": generation, "offset": offset,
                            "xref_entry_offset": entry_position})
    require([entry["object"] for entry in entries] == [2, 5, 7, 8, 24],
            "xref proof: latest section differs from requested object changes")
    require(all(entry["generation"] == 0 for entry in entries), "xref proof: generation changed")
    require([entry["offset"] for entry in entries] == sorted({entry["offset"] for entry in entries}),
            "xref proof: duplicate or unordered object offsets")
    tail = stream.read()
    trailer = re.fullmatch(rb"(<<[^<>]*?(?:<[^<>]*>[^<>]*?)*>>)\nstartxref\n(\d+)\n%%EOF\s*", tail)
    require(trailer is not None and int(trailer[2]) == current, "xref proof: malformed latest trailer/tail")
    prev_values = re.findall(rb"/Prev\s+(\d+)(?=\s|/|>>)", trailer[1])
    require(prev_values == [str(previous).encode("ascii")], "xref proof: Prev does not identify the exact source xref")
    require(candidate[previous:len(source)] == source[previous:], "xref proof: previous xref/trailer changed")
    reader_source = Path(pypdf._reader.__file__)
    return {"result": "passed", "check_scope": "Frozen original checkbox fixture; raw classic xref append, source prefix, Prev and exact object headers",
            "source_xref_offset": previous, "latest_xref_offset": current,
            "first_subsection": subsection_starts[0], "entries": entries,
            "reader_behavior_source": {"package": "pypdf", "version": pypdf.__version__,
                                       "file": "pypdf/_reader.py", "sha256": sha(reader_source.read_bytes()),
                                       "warning_method": "PdfReader._read_standard_xref_table: first_time and num != 0 and self.strict",
                                       "correction_condition": "PdfReader.read: self.xref_index and not self.strict",
                                       "url": "https://github.com/py-pdf/pypdf/blob/6.10.0/pypdf/_reader.py"}}


def assess_reader_warnings(phase, warnings, source, candidate=None):
    if not warnings:
        return {"phase": phase, "warnings": [], "result": "passed"}
    require(phase == "incremental" and all(warning == SPARSE_XREF_WARNING for warning in warnings),
            f"unacknowledged independent reader warning in {phase}: {warnings}")
    proof = sparse_incremental_xref_proof(source, candidate)
    return {"phase": phase, "warnings": warnings, "result": "acknowledged_with_raw_xref_proof", "proof": proof}


def warning_gate_controls(source, candidate):
    proof = sparse_incremental_xref_proof(source, candidate)
    first = proof["entries"][0]
    offset = first["xref_entry_offset"]
    bad_offset = candidate[:offset] + f"{first['offset'] + 1:010}".encode("ascii") + candidate[offset + 10:]
    header = first["offset"]
    bad_header = candidate[:header] + b"9" + candidate[header + 1:]
    previous = f"/Prev {proof['source_xref_offset']}".encode("ascii")
    latest = proof["latest_xref_offset"]
    require(candidate[latest:].count(previous) == 1, "warning control: ambiguous Prev")
    bad_previous = candidate[:latest] + candidate[latest:].replace(
        previous, f"/Prev {proof['source_xref_offset'] + 1}".encode("ascii"), 1)
    controls = {}
    for name, data, warnings in [
            ("bad_xref_offset", bad_offset, [SPARSE_XREF_WARNING]),
            ("mismatched_object_header", bad_header, [SPARSE_XREF_WARNING]),
            ("wrong_previous_revision", bad_previous, [SPARSE_XREF_WARNING]),
            ("unknown_reader_warning", candidate, [SPARSE_XREF_WARNING, "Injected unknown reader warning"]),
            ("sparse_warning_on_compact", candidate, [SPARSE_XREF_WARNING])]:
        phase = "compact" if name == "sparse_warning_on_compact" else "incremental"
        try:
            assess_reader_warnings(phase, warnings, source, data)
        except RuntimeError as error:
            controls[name] = {"detected": str(error), "sha256": sha(data), "gate": "assess_reader_warnings",
                              "input_warnings": warnings, "phase": phase}
        else:
            raise RuntimeError(f"warning gate accepted {name}")
    return controls


def fixture_bytes():
    stream = storage.stream
    normal = b"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 24 24] /Matrix [1 0 0 1 0 0] /Resources << >>"
    box = b"q 1 1 1 rg 0 0 24 24 re f 0.15 0.2 0.3 RG 1.2 w 1 1 22 22 re S Q\n"
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R /AcroForm 15 0 R /Metadata 16 0 R /Outlines 17 0 R /Names 19 0 R /PageLabels 24 0 R /Lang (en-US) /Private (original catalog) >>",
        2: b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 320 240] /Resources 25 0 R >>",
        3: b"<< /Type /Page /Parent 2 0 R /Contents 13 0 R /Annots [5 0 R 23 0 R] /Private (accept page) >>",
        4: b"<< /Type /Page /Parent 2 0 R /Contents 14 0 R /Annots [8 0 R] /Private (subscribe page) >>",
        5: b"<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 0 /T (accept) /TU (Accept terms) /V /Off /DV /Off /AS /Off /Rect [30 140 54 164] /P 3 0 R /F 4 /AP << /N << /Off 11 0 R /Yes 12 0 R >> >> /MK << /BC [0.15 0.2 0.3] /BG [1 1 1] >> /Private (combined checkbox) >>",
        6: b"<< /T (preferences) /Kids [7 0 R] /Private (field group) >>",
        7: b"<< /Parent 6 0 R /T (subscribe) /FT /Btn /Ff 0 /V /Yes /DV /Yes /TU (Subscribe to updates) /Kids [8 0 R] /Private (terminal checkbox) >>",
        8: b"<< /Type /Annot /Subtype /Widget /Parent 7 0 R /P 4 0 R /Rect [30 140 54 164] /F 4 /AS /Yes /AP << /N << /Off 11 0 R /Yes 12 0 R >> >> /Private (separate widget) >>",
        9: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 10 0 R >>",
        10: stream(b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /OriginalASCII def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfrange <20> <7E> <0020> endbfrange endcmap CMapName currentdict /CMap defineresource pop end end"),
        11: stream(box, normal),
        12: stream(box + b"q 0.05 0.45 0.2 RG 3 w 1 J 1 j 5 12 m 10 7 l 19 18 l S Q\n", normal),
        13: stream(b"BT /F1 19 Tf 30 204 Td (ACCEPT TERMS) Tj /F1 12 Tf 0 -82 Td (Accept the original terms) Tj 0 -46 Td (Go to subscription preferences) Tj ET\n"),
        14: stream(b"BT /F1 19 Tf 30 204 Td (SUBSCRIPTION) Tj /F1 12 Tf 0 -82 Td (Receive optional updates) Tj 0 -46 Td (Your existing setting is retained until commit.) Tj ET\n"),
        15: b"<< /Fields [5 0 R 6 0 R] /NeedAppearances false /DA (/F1 12 Tf 0 g) /DR << /Font << /F1 9 0 R >> >> /Private (form registration) >>",
        16: stream(b'<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Original checkbox states fixture</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>', b"/Type /Metadata /Subtype /XML"),
        17: b"<< /Type /Outlines /First 18 0 R /Last 18 0 R /Count 1 >>",
        18: b"<< /Title (Subscription preferences) /Parent 17 0 R /Dest [4 0 R /Fit] >>",
        19: b"<< /Dests << /Names [(subscribe) [4 0 R /Fit]] >> /EmbeddedFiles << /Names [(note.txt) 20 0 R] >> >>",
        20: b"<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F 21 0 R >> /Desc (Original exact attachment) >>",
        21: stream(ATTACHMENT, b"/Type /EmbeddedFile /Subtype /text#2Fplain"),
        22: b"<< /Title (Original checkbox states fixture) /Author (Binas original Apache-2.0 QA) >>",
        23: b"<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [30 70 205 92] /Dest [4 0 R /Fit] /Border [0 0 0] >>",
        24: b"<< /Nums [0 << /S /D /P (A-) /St 1 /Private (accept label) >> 1 << /S /D /P (B-) /St 1 /Private (subscribe label) >>] /Private (label tree) >>",
        25: b"<< /Font << /F1 9 0 R >> /ProcSet [/PDF /Text] >>",
    }
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for number, value in sorted(objects.items()):
        offsets[number] = len(output)
        output += f"{number} 0 obj\n".encode() + value + b"\nendobj\n"
    xref = len(output)
    output += b"xref\n0 26\n0000000000 65535 f \n"
    for number in range(1, 26):
        output += f"{offsets[number]:010} 00000 n \n".encode()
    output += ("trailer\n<< /Size 26 /Root 1 0 R /Info 22 0 R "
               "/ID [<434845434B424F582D5354415445533031> <434845434B424F582D5354415445533031>] >>\n"
               f"startxref\n{xref}\n%%EOF\n").encode()
    return bytes(output)


def complete_inventory(data):
    reader, objects = storage.inventory(data)
    return {
        "objects": {f"{number}:{generation}": storage.typed(value)
                    for (number, generation), value in objects.items()},
        "encoded_streams": {f"{number}:{generation}": sha(value._data)
                            for (number, generation), value in objects.items()
                            if isinstance(value, StreamObject)},
        "trailer": storage.trailer(reader),
    }


def expected_inventory(source):
    def requested(objects):
        objects[(2, 0)][NameObject("/Kids")] = ArrayObject([
            objects[(4, 0)].indirect_reference, objects[(3, 0)].indirect_reference])
        objects[(5, 0)][NameObject("/V")] = NameObject("/Yes")
        objects[(5, 0)][NameObject("/AS")] = NameObject("/Yes")
        objects[(7, 0)][NameObject("/V")] = NameObject("/Off")
        objects[(8, 0)][NameObject("/AS")] = NameObject("/Off")
        nums = objects[(24, 0)]["/Nums"]
        objects[(24, 0)][NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[3], NumberObject(1), nums[1]])
    return complete_inventory(geometry.rewrite_objects(source, requested))


def inspect(data):
    reader, objects = storage.inventory(data)
    root = reader.trailer["/Root"]
    fields = reader.get_fields()
    require(fields is not None and set(REQUESTS).issubset(fields), "missing AcroForm field registration")
    page_ids = [page.indirect_reference.idnum for page in reader.pages]
    states = {}
    for name, field, widget, page in [("accept", 5, 5, 3), ("preferences.subscribe", 7, 8, 4)]:
        field_object, widget_object = objects[(field, 0)], objects[(widget, 0)]
        require(fields[name].get("/V") == field_object["/V"], f"registered value disagrees: {name}")
        require(widget_object["/AS"] == field_object["/V"], f"field/widget state mismatch: {name}")
        require(widget_object.raw_get("/P").idnum == page, f"widget page reference changed: {name}")
        page_object = objects[(page, 0)]
        require(widget in [item.idnum for item in page_object["/Annots"]], f"widget missing from page: {name}")
        normal = widget_object["/AP"]["/N"]
        require(set(normal) == {"/Off", "/Yes"}, f"appearance states changed: {name}")
        require([normal.raw_get(state).idnum for state in ["/Off", "/Yes"]] == [11, 12], "immutable shared appearances changed")
        states[name] = {"field": field, "widget": widget, "page": page,
                        "current_page_index": page_ids.index(page), "value": str(field_object["/V"]),
                        "appearance_state": str(widget_object["/AS"])}
    require([item.idnum for item in root["/AcroForm"]["/Fields"]] == [5, 6], "top-level field roots changed")
    require([item.idnum for item in objects[(6, 0)]["/Kids"]] == [7], "nested field registration changed")
    require([item.idnum for item in objects[(7, 0)]["/Kids"]] == [8], "separate widget registration changed")
    require(objects[(7, 0)].raw_get("/Parent").idnum == 6
            and objects[(8, 0)].raw_get("/Parent").idnum == 7, "field/widget parents changed")
    return {"page_ids": page_ids, "states": states, "labels": reader.page_labels,
            "text": [page.extract_text() for page in reader.pages],
            "named_destinations": {name: reader.get_destination_page_number(value)
                                   for name, value in reader.named_destinations.items()},
            "outline_destinations": [reader.get_destination_page_number(value) for value in reader.outline],
            "attachments": {name: [sha(value) for value in values] for name, values in reader.attachments.items()}}


def source_checks(source):
    result = inspect(source)
    require(result["page_ids"] == [3, 4] and result["labels"] == ["A-1", "B-1"], "frozen source page identity/labels changed")
    require([result["states"][name]["value"] for name in REQUESTS] == ["/Off", "/Yes"], "source checkbox states changed")
    require(result["attachments"] == {"note.txt": [sha(ATTACHMENT)]}, "source attachment changed")
    require("ACCEPT TERMS" in result["text"][0] and "SUBSCRIPTION" in result["text"][1], "source visible page text missing")
    return result


def verify(source, candidate):
    before = source_checks(source)
    expected, actual = expected_inventory(source), complete_inventory(candidate)
    require(set(actual["objects"]) == set(expected["objects"]), "object identities added or removed")
    for identity, value in expected["objects"].items():
        require(actual["objects"][identity] == value, f"unrequested typed object change: {identity}")
    require(actual["encoded_streams"] == expected["encoded_streams"], "encoded appearance/content/asset bytes changed")
    require(actual["trailer"] == expected["trailer"], "meaningful trailer changed")
    after = inspect(candidate)
    require(after["page_ids"] == [4, 3], "requested page order missing")
    require({name: value["value"] for name, value in after["states"].items()} == REQUESTS, "requested checkbox states missing")
    for key in ["labels", "text"]:
        require(after[key] == [before[key][index] for index in ORDER], f"page {key} did not follow original identity")
    require(after["named_destinations"] == {name: ORDER.index(index) for name, index in before["named_destinations"].items()}, "named destinations changed")
    require(after["outline_destinations"] == [ORDER.index(index) for index in before["outline_destinations"]], "outline destinations changed")
    require(after["attachments"] == before["attachments"], "attachment bytes changed")
    return {"state_inventory": after, "complete_inventory": actual}


def negative_controls(source, candidate):
    def wrong_state(objects):
        for key in ["/V", "/AS"]:
            objects[(5, 0)][NameObject(key)] = NameObject("/Off")
    def wrong_appearance(objects):
        objects[(5, 0)][NameObject("/AS")] = NameObject("/Off")
    def missing_registration(objects):
        fields = objects[(15, 0)]["/Fields"]
        objects[(15, 0)][NameObject("/Fields")] = ArrayObject([fields[1]])
    def changed_appearance_stream(objects):
        objects[(12, 0)].set_data(objects[(11, 0)].get_data())
    results, outputs = {}, {}
    for name, change in [("wrong_state", wrong_state), ("wrong_appearance_state", wrong_appearance),
                         ("missing_registration", missing_registration), ("changed_appearance_stream", changed_appearance_stream)]:
        corrupted = geometry.rewrite_objects(candidate, change)
        require(len(pypdf.PdfReader(io.BytesIO(corrupted), strict=True).pages) == 2, f"negative {name} cannot reparse")
        try:
            verify(source, corrupted)
        except RuntimeError as error:
            results[name] = {"reparse": "passed", "detected": str(error), "sha256": sha(corrupted)}
        else:
            raise RuntimeError(f"oracle accepted {name} corruption")
        outputs[name] = corrupted
    return results, outputs


def independent_expected_pdf(source):
    writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(source), strict=True))
    changed = set()
    def field_states(reference, prefix=""):
        field = reference.get_object()
        name = ".".join(item for item in [prefix, str(field.get("/T", ""))] if item)
        if name in REQUESTS:
            require(field.get("/FT") == "/Btn", "expected author selected a non-button")
            state = NameObject(REQUESTS[name])
            field[NameObject("/V")] = state
            widget = field if field.get("/Subtype") == "/Widget" else field["/Kids"][0].get_object()
            require(widget.get("/Subtype") == "/Widget", "expected author selected a non-widget")
            widget[NameObject("/AS")] = state
            changed.add(name)
            return
        for child in field.get("/Kids", []):
            field_states(child, name)
    for field in writer.root_object["/AcroForm"]["/Fields"]:
        field_states(field)
    require(changed == set(REQUESTS), "expected author missed a requested named field")
    pages = list(writer.pages)
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
    reopened = pypdf.PdfReader(io.BytesIO(expected), strict=True)
    require(len(reopened.pages) == 2, "independent expected page count changed")
    require({name: str(reopened.get_fields()[name]["/V"]) for name in REQUESTS} == REQUESTS, "independent expected field values changed")
    require([page.extract_text() for page in reopened.pages] == [source_checks(source)["text"][index] for index in ORDER], "independent expected text/order changed")
    return expected


def render_checks(tool, source, paths, negatives, report):
    from PIL import Image, ImageChops, __version__ as pillow_version
    tool = tool.resolve(strict=True)
    work = REPO / "target/binas-validation/checkbox"
    work.mkdir(parents=True, exist_ok=True)
    expected = independent_expected_pdf(source)
    expected_path = work / "independent-expected.pdf"
    expected_path.write_bytes(expected)
    rendering_inputs = {"expected": expected_path, **paths}
    for name in ["wrong_state", "changed_appearance_stream"]:
        path = work / f"negative-{name}.pdf"
        path.write_bytes(negatives[name])
        rendering_inputs[name] = path
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    images, records = {}, []
    for name, path in rendering_inputs.items():
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
                   for name in rendering_inputs if name != "expected"}
    for name in paths:
        require(comparisons[name] == [True, True], f"{name} differs from independent checkbox RGB144 expected rendering")
    for name in ["wrong_state", "changed_appearance_stream"]:
        require(comparisons[name] == [True, False], f"{name} was not detected only on the accept page")
    report["rendering"] = {"result": "passed", "color": "RGB", "dpi": 144, "all_pages": True,
                           "expected_pdf_sha256": sha(expected), "expected_authorship": "pypdf source clone, field-name state assignment, explicit page reorder",
                           "pillow": pillow_version, "poppler": (version.stdout + version.stderr).strip(),
                           "renderer_sha256": sha(tool.read_bytes()), "comparisons": comparisons, "renders": records}


def generate_fixture():
    source = fixture_bytes()
    source_checks(source)
    require(not oracle.WARNINGS and not storage.WARNINGS, "source emitted reader warnings")
    SOURCE.write_bytes(source)
    PROVENANCE.write_text(json.dumps({
        "schema": 1, "license": "Apache-2.0", "origin": "Original Binas synthetic checkbox fixture; no external document or artwork assets.",
        "external_assets": [], "path": "fixtures/checkbox-states.pdf", "bytes": len(source), "sha256": sha(source),
        "generator": "checkbox_oracle.py --generate-fixture", "generator_sha256": sha(Path(__file__).read_bytes()),
        "oracle_sha256": sha(Path(__file__).read_bytes()),
        "python": platform.python_version(), "pypdf": pypdf.__version__, "page_objects": [3, 4],
        "combined_field_widget": 5, "nested_parent": 6, "terminal_field": 7, "separate_widget": 8,
        "shared_normal_appearances": {"Off": 11, "Yes": 12}, "acroform": 15, "source_states": {"accept": "Off", "preferences.subscribe": "Yes"},
        "request": {"states": REQUESTS, "page_order": ORDER},
        "retained_assets": {"metadata": 16, "outline": 18, "names": 19, "attachment": 21, "link": 23, "labels": 24},
        "assurance": "Source strict pypdf checks only. Engine outputs and RGB comparisons require a separate successful gate run."
    }, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"fixture_sha256": sha(source), "bytes": len(source), "source_reader": "passed"}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--generate-fixture", action="store_true")
    mode.add_argument("--structure-only", action="store_true")
    mode.add_argument("--pdftoppm", type=Path)
    args = parser.parse_args()
    require(pypdf.__version__ == "6.10.0", "install pinned pypdf 6.10.0")
    if args.generate_fixture:
        generate_fixture()
        return
    source = SOURCE.read_bytes()
    provenance = json.loads(PROVENANCE.read_text(encoding="utf-8"))
    require(sha(source) == provenance["sha256"], "frozen checkbox source hash mismatch")
    require(source == fixture_bytes(), "current generator no longer matches the frozen checkbox bytes")
    require(sha(Path(__file__).read_bytes()) == provenance["oracle_sha256"], "checkbox oracle changed; explicitly review its separate provenance hash")
    require(not oracle.WARNINGS and not storage.WARNINGS, "reader warnings occurred before scoped gate phases")
    warning_phases = []
    checkpoint = warning_checkpoint()
    source_checks(source)
    warning_phases.append(assess_reader_warnings("source", warnings_since(checkpoint), source))
    paths = {name: ROOT / f"evidence/checkbox-{name}.pdf" for name in ["incremental", "compact"]}
    candidates = {name: path.read_bytes() for name, path in paths.items()}
    outputs = []
    for name, candidate in candidates.items():
        checkpoint = warning_checkpoint()
        if name == "incremental":
            require(candidate.startswith(source), "incremental output lost exact input prefix")
        outputs.append({"mode": name, "bytes": len(candidate), "sha256": sha(candidate), "checks": verify(source, candidate)})
        warning_phases.append(assess_reader_warnings(name, warnings_since(checkpoint), source, candidate))
    checkpoint = warning_checkpoint()
    controls, negatives = negative_controls(source, candidates["compact"])
    warning_phases.append(assess_reader_warnings("corruption_controls", warnings_since(checkpoint), source))
    warning_controls = warning_gate_controls(source, candidates["incremental"])
    report = {"schema": 1, "result": "passed", "scope": __doc__.strip(),
              "python": platform.python_version(), "pypdf": pypdf.__version__, "source_sha256": sha(source),
              "oracle_sha256": sha(Path(__file__).read_bytes()), "provenance_sha256": sha(PROVENANCE.read_bytes()),
              "example_sha256": sha((REPO / "crates/binas-pdf/examples/preservation_checkbox.rs").read_bytes()),
              "helper_sha256": {name: sha((ROOT / name).read_bytes()) for name in ["flate_oracle.py", "geometry_oracle.py", "oracle.py", "requirements.txt", "LICENSE"]},
              "request": {"states": REQUESTS, "page_order": ORDER}, "outputs": outputs, "negative_controls": controls,
              "warning_gate_controls": warning_controls,
              "ignored_layout": ["stream Length representation", "xref offsets and representation", "trailer Size/Prev/XRefStm"],
              "rendering": {"result": "not_checked"}}
    if args.pdftoppm:
        checkpoint = warning_checkpoint()
        render_checks(args.pdftoppm, source, paths, negatives, report)
        warning_phases.append(assess_reader_warnings("independent_render_expectation", warnings_since(checkpoint), source))
    recorded_warnings = [warning for phase in warning_phases for warning in phase["warnings"]]
    require(recorded_warnings == oracle.WARNINGS == storage.WARNINGS, "reader warnings escaped scoped phase accounting")
    report["reader_warnings"] = warning_phases
    receipt = "checkbox-evidence.json" if args.pdftoppm else "checkbox-structure.json"
    (ROOT / "evidence" / receipt).write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "writer_outputs": 2, "corruption_controls": len(controls), "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
