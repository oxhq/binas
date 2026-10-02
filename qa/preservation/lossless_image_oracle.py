"""Original lossless image predictor fixture and independent pypdf/RGB gate.

Exact 8-bit RGB/Gray samples move from raw Flate to PNG Up12 after a required
text edit. Shared image uses, inverse Gray Decode, original soft-mask bytes,
navigation and opaque assets remain unchanged. This synthetic evidence is not
general image/codec/color compatibility, compression performance or conformance.
"""
import argparse
import io
import json
import platform
import subprocess
import zlib
from pathlib import Path

import pypdf
from pypdf.generic import ArrayObject, DictionaryObject, NameObject, NumberObject, StreamObject

import flate_oracle as storage
import flatten_oracle as flatten
import geometry_oracle as geometry
import oracle
import text_field_oracle as text_fields

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
SOURCE = ROOT / "fixtures/lossless-images.pdf"
PROVENANCE = ROOT / "fixtures/lossless-images.provenance.json"
WIDTH, HEIGHT = 96, 64
ATTACHMENT = b"Original lossless image attachment; retain NUL:\x00 and bytes:\xff\x01\n"
TINY = bytes([13, 241, 71, 129, 7, 203, 53, 149, 229, 31, 181, 97])
sha, require = storage.sha, storage.require


def samples(colors):
    return bytes((x*(17+12*c)+y*(3+2*c)) % 256
                 for y in range(HEIGHT) for x in range(WIDTH) for c in range(colors))


def mask_samples():
    return bytes(80+(x*2+y) % 176 for y in range(HEIGHT) for x in range(WIDTH))


def up_rows(data, columns, colors):
    row_bytes = columns*colors
    require(len(data) % row_bytes == 0, "independent predictor has a partial row")
    return b"".join(b"\x02" + bytes((data[start+i]-(data[start-row_bytes+i] if start else 0)) % 256
                                   for i in range(row_bytes))
                    for start in range(0, len(data), row_bytes))


def stream(data, extra=b""):
    return b"<< /Length " + str(len(data)).encode() + b" " + extra + b" >>\nstream\n" + data + b"\nendstream"


def fixture_bytes():
    image = b"/Type /XObject /Subtype /Image /Width 96 /Height 64 /BitsPerComponent 8 /Interpolate false"
    background = (b"q 0.92 0.84 0.72 rg 20 110 120 90 re f 0.15 0.3 0.65 rg 20 110 60 45 re f "
                  b"80 155 60 45 re f Q\n")
    content_a = (b"BT /F1 19 Tf 20 230 Td (before) Tj ET\n" + background
                 + b"q 120 0 0 80 20 110 cm /Rgb Do Q\nq 120 0 0 80 160 110 cm /Gray Do Q\nq 24 0 0 24 20 50 cm /Tiny Do Q\n")
    content_b = (b"BT /F1 19 Tf 20 230 Td (untouched) Tj ET\n" + background
                 + b"q 1 0 0 1 20 110 cm /Nested Do Q\nq 100 0 0 66.6666667 170 50 cm /Gray Do Q\n")
    cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /OriginalASCII def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfrange <20> <7E> <0020> endbfrange endcmap CMapName currentdict /CMap defineresource pop end end"
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R /Metadata 14 0 R /Outlines 15 0 R /Names 17 0 R /PageLabels 21 0 R /Lang (en-US) /Private (original catalog) >>",
        2: b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 260] /Resources 11 0 R /Private (shared page resources) >>",
        3: b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Private (alpha page) >>",
        4: b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Private (bravo page) >>",
        5: stream(content_a), 6: stream(content_b),
        7: stream(zlib.compress(samples(3), 9), image + b" /ColorSpace /DeviceRGB /Filter /FlateDecode /SMask 9 0 R /Private (shared RGB samples)"),
        8: stream(zlib.compress(samples(1), 9), image + b" /ColorSpace /DeviceGray /Filter /FlateDecode /Decode [1 0] /Private (inverse Gray samples)"),
        9: stream(mask_samples(), image + b" /ColorSpace /DeviceGray /Private (unfiltered soft mask retained exactly)"),
        10: stream(b"q 120 0 0 80 0 0 cm /Rgb Do Q\n", b"/Type /XObject /Subtype /Form /BBox [0 0 120 80] /Matrix [1 0 0 1 0 0] /Resources << /XObject << /Rgb 7 0 R >> >> /Private (nested shared RGB use)"),
        11: b"<< /Font << /F1 12 0 R >> /XObject << /Rgb 7 0 R /Gray 8 0 R /Nested 10 0 R /Tiny 13 0 R >> /ProcSet [/PDF /Text /ImageB /ImageC] /Private (shared inherited resources) >>",
        12: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 22 0 R >>",
        13: stream(zlib.compress(TINY, 9), b"/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /FlateDecode /Interpolate false /Private (original wins against Up)"),
        14: stream(b'<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Original lossless images</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>', b"/Type /Metadata /Subtype /XML"),
        15: b"<< /Type /Outlines /First 16 0 R /Last 16 0 R /Count 1 >>",
        16: b"<< /Title (Bravo page) /Parent 15 0 R /Dest [4 0 R /Fit] >>",
        17: b"<< /Dests << /Names [(bravo) [4 0 R /Fit]] >> /EmbeddedFiles << /Names [(note.txt) 18 0 R] >> >>",
        18: b"<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F 19 0 R >> /Desc (Exact original attachment) >>",
        19: stream(ATTACHMENT, b"/Type /EmbeddedFile /Subtype /text#2Fplain"),
        20: b"<< /Title (Original lossless images) /Author (Binas original Apache-2.0 QA) >>",
        21: b"<< /Nums [0 << /S /D /P (A-) /St 1 >> 1 << /S /D /P (B-) /St 1 >>] /Private (explicit labels) >>",
        22: stream(cmap),
    }
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for number, value in objects.items():
        offsets[number] = len(output)
        output += f"{number} 0 obj\n".encode() + value + b"\nendobj\n"
    xref = len(output)
    output += b"xref\n0 23\n0000000000 65535 f \n"
    for number in range(1, 23):
        output += f"{offsets[number]:010} 00000 n \n".encode()
    output += ("trailer\n<< /Size 23 /Root 1 0 R /Info 20 0 R /ID [<4C4F53534C455353494D41474553303131> <4C4F53534C455353494D41474553303131>] >>\n"
               f"startxref\n{xref}\n%%EOF\n").encode()
    return bytes(output)


def inspect(data):
    reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
    return {"pages": [page.indirect_reference.idnum for page in reader.pages], "labels": reader.page_labels,
            "text": [page.extract_text() for page in reader.pages],
            "destinations": {name: reader.get_destination_page_number(value) for name, value in reader.named_destinations.items()},
            "outline": [reader.get_destination_page_number(value) for value in reader.outline],
            "attachments": {name: [sha(value) for value in values] for name, values in reader.attachments.items()}}


def source_checks(source):
    inventory = inspect(source)
    require(inventory == {"pages": [3, 4], "labels": ["A-1", "B-1"], "text": ["before\n", "untouched\n"],
                          "destinations": {"bravo": 1}, "outline": [1], "attachments": {"note.txt": [sha(ATTACHMENT)]}}, "source inventory changed")
    _, objects = storage.inventory(source)
    for number, expected in [(7, samples(3)), (8, samples(1)), (9, mask_samples()), (13, TINY)]:
        require(objects[(number, 0)].get_data() == expected, f"source image samples changed: {number}")
    require(len(zlib.compress(up_rows(TINY, 2, 3), 9)) > len(objects[(13, 0)]._data), "tiny source no longer demonstrates original fallback")
    return inventory


def dictionary_except(value, ignored):
    return {str(key): storage.typed(item) for key, item in value.items() if key not in ignored}


def verify_baseline(source, baseline):
    expected = source_checks(source)
    actual = inspect(baseline)
    require(actual == {**expected, "text": ["after\n", "untouched\n"]}, "required edited baseline inventory differs")
    old_reader, old = storage.inventory(source)
    reader, new = storage.inventory(baseline)
    require(set(old) == set(new) and storage.trailer(old_reader) == storage.trailer(reader), "baseline object identities/trailer changed")
    for ref, value in old.items():
        if ref == (5, 0):
            require(dictionary_except(value, {"/Length"}) == dictionary_except(new[ref], {"/Length"}), "text edit changed stream dictionary")
            text_fields.verify_token_replacement(value.get_data(), new[ref].get_data(), "before", "after", reader)
        else:
            require(storage.typed(value) == storage.typed(new[ref]), f"baseline changed unrelated object {ref}")
            if isinstance(value, StreamObject):
                require(value._data == new[ref]._data, f"baseline changed unrelated encoded stream {ref}")
    return actual


def predictor_dictionary(colors):
    return DictionaryObject({NameObject(key): NumberObject(value) for key, value in
                             [("/Predictor", 12), ("/Colors", colors), ("/BitsPerComponent", 8), ("/Columns", WIDTH)]})


def verify(baseline, candidate):
    require(inspect(candidate) == inspect(baseline), "optimization changed edited page/document inventory")
    old_reader, old = storage.inventory(baseline)
    _, new = storage.inventory(candidate)
    require(set(old) == set(new), "optimization added or removed objects")
    require(storage.trailer(old_reader) == storage.trailer(pypdf.PdfReader(io.BytesIO(candidate), strict=True)), "meaningful trailer changed")
    effects = []
    for ref, before in old.items():
        after = new[ref]
        if ref not in {(7, 0), (8, 0)}:
            require(storage.typed(before) == storage.typed(after), f"unrequested object change {ref}")
            if isinstance(before, StreamObject):
                require(before._data == after._data, f"unrequested encoded stream change {ref}")
            continue
        colors = 3 if ref[0] == 7 else 1
        expected = samples(colors)
        require(isinstance(after, StreamObject), "image stream replaced with nonstream")
        require(dictionary_except(before, {"/Length", "/DecodeParms"}) == dictionary_except(after, {"/Length", "/DecodeParms"}), f"image interpretation dictionary changed {ref}")
        require(storage.typed(after.get("/DecodeParms")) == storage.typed(predictor_dictionary(colors)), f"image predictor geometry differs {ref}")
        require(before.get_data() == expected == after.get_data(), f"decoded image samples differ {ref}")
        # This independent arithmetic also checks every encoded row marker and
        # delta; pypdf separately reverses the candidate predictor/filter chain.
        require(zlib.decompress(after._data) == up_rows(expected, WIDTH, colors), f"PNG Up rows differ {ref}")
        require(len(after._data) < len(before._data), f"candidate image payload is not smaller {ref}")
        effects.append({"image": list(ref), "width": WIDTH, "height": HEIGHT, "colors": colors,
                        "decoded_bytes": len(expected), "samples_sha256": sha(expected), "source_predictor": 1, "output_predictor": 12,
                        "encoded_before": len(before._data), "encoded_after": len(after._data)})
    require(len(effects) == 2, "expected two image candidates")
    return {"objects": len(old), "effects": effects, "edited_text": inspect(candidate)["text"],
            "unchanged_mask_sha256": sha(new[(9, 0)]._data), "tiny_original_sha256": sha(new[(13, 0)]._data),
            "shared_placements": {"RGB7": ["page3 direct", "page4 Form10"], "Gray8": ["page3 direct", "page4 direct"]},
            "encoded_bytes_saved": sum(effect["encoded_before"]-effect["encoded_after"] for effect in effects),
            "baseline_pdf_bytes": len(baseline), "candidate_pdf_bytes": len(candidate), "final_bytes_saved": len(baseline)-len(candidate)}


def independent_baseline(source):
    writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(source), strict=True))
    contents = writer.pages[0]["/Contents"]
    require(contents._data.count(b"(before) Tj") == 1, "independent selected text missing")
    contents._data = contents._data.replace(b"(before) Tj", b"(after) Tj")
    output = io.BytesIO()
    writer.write(output)
    return output.getvalue()


def identity_baseline(source):
    def edit(objects):
        contents = objects[(5, 0)]
        contents._data = contents._data.replace(b"(before) Tj", b"(after) Tj")
    return geometry.rewrite_objects(source, edit)


def independent_candidate(baseline):
    def predict(objects):
        for number, colors in [(7, 3), (8, 1)]:
            objects[(number, 0)][NameObject("/DecodeParms")] = predictor_dictionary(colors)
            objects[(number, 0)]._data = zlib.compress(up_rows(samples(colors), WIDTH, colors), 9)
    return geometry.rewrite_objects(baseline, predict)


def negative_controls(baseline, candidate):
    def pixel(objects):
        value = bytearray(samples(3))
        value[(20*WIDTH+30)*3] ^= 128
        objects[(7, 0)]._data = zlib.compress(up_rows(value, WIDTH, 3), 9)
    def gray_decode(objects):
        del objects[(8, 0)]["/Decode"]
    def mask(objects):
        del objects[(7, 0)]["/SMask"]
    def color(objects):
        objects[(7, 0)][NameObject("/ColorSpace")] = NameObject("/DeviceCMYK")
    def geometry_control(objects):
        params = objects[(7, 0)]["/DecodeParms"]
        # Keep the same byte stride and valid Up rows so this remains decodable,
        # while contradicting image width/components and the declared contract.
        params[NameObject("/Columns")] = NumberObject(WIDTH*3)
        params[NameObject("/Colors")] = NumberObject(1)
    def placement(objects):
        objects[(10, 0)]._data = b"q Q\n"
    def attachment(objects):
        objects[(19, 0)]._data = b"Unrequested replacement attachment"
    def metadata(objects):
        objects[(14, 0)]._data = objects[(14, 0)]._data.replace(b"Original lossless images", b"Unrequested new title")
    def rollback(objects):
        objects[(5, 0)]._data = storage.inventory(SOURCE.read_bytes())[1][(5, 0)]._data
    _, original = storage.inventory(candidate)
    original_typed = {ref: storage.typed(value) for ref, value in original.items()}
    controls, negatives = {}, {}
    for name, mutate in [("one_pixel_changed", pixel), ("gray_decode_removed", gray_decode), ("soft_mask_removed", mask),
                         ("color_space_changed", color), ("predictor_geometry_changed", geometry_control), ("shared_placement_removed", placement),
                         ("attachment_changed", attachment), ("metadata_changed", metadata), ("required_edit_discarded", rollback)]:
        corrupted = geometry.rewrite_objects(candidate, mutate)
        require(len(pypdf.PdfReader(io.BytesIO(corrupted), strict=True).pages) == 2, f"control {name} cannot reparse")
        _, changed_objects = storage.inventory(corrupted)
        changed = [list(ref) for ref, value in changed_objects.items() if storage.typed(value) != original_typed[ref]]
        require(len(changed) == 1, f"control {name} did not change exactly one object")
        try:
            verify(baseline, corrupted)
        except RuntimeError as error:
            controls[name] = {"reparse": "passed", "changed_objects": changed, "detected": str(error), "sha256": sha(corrupted)}
        else:
            raise RuntimeError(f"oracle accepted {name}")
        negatives[name] = corrupted
    return controls, negatives


def render_checks(tool, source, paths, negatives, report):
    from PIL import Image, ImageChops, __version__ as pillow_version
    tool = tool.resolve(strict=True)
    work = REPO / "target/binas-validation/lossless-images"
    work.mkdir(parents=True, exist_ok=True)
    expected = independent_baseline(source)
    expected_path = work / "independent-edited-expected.pdf"
    expected_path.write_bytes(expected)
    inputs = {"expected": expected_path, **paths}
    visual_controls = {"one_pixel_changed": [False, False], "gray_decode_removed": [False, False], "soft_mask_removed": [False, False],
                       "shared_placement_removed": [True, False], "required_edit_discarded": [False, True]}
    for name in visual_controls:
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
            # This pinned Windows Poppler bundle reports absent optional display
            # fonts during initialization, including on the independent source
            # clone. Only these exact diagnostics are acknowledged; no arbitrary
            # syntax/rendering errors are accepted as visual evidence.
            known_font_diagnostics = {"Syntax Error: No display font for 'Symbol'", "Syntax Error: No display font for 'ArialUnicode'"}
            require(set(result.stderr.splitlines()) <= known_font_diagnostics,
                    f"unacknowledged renderer diagnostic for {name}: {result.stderr}")
            png = Path(str(prefix)+".png")
            with Image.open(png) as opened:
                image = opened.convert("RGB")
            images[name].append(image)
            records.append({"name": name, "page": page, "command": command, "size": list(image.size), "stderr": result.stderr,
                            "rgb_sha256": sha(image.tobytes()), "png_sha256": sha(png.read_bytes())})
    comparisons = {name: [a.size == b.size and ImageChops.difference(a, b).getbbox() is None
                         for a, b in zip(images["expected"], images[name])] for name in inputs if name != "expected"}
    for name in paths:
        require(comparisons[name] == [True, True], f"{name} differs from independent edited RGB expected")
    for name, expected_match in visual_controls.items():
        require(comparisons[name] == expected_match, f"{name} did not change exactly the expected pages")
    diagnostics = {record["stderr"] for record in records}
    require(len(diagnostics) == 1, "renderer diagnostics differ from independent expectation")
    report["rendering"] = {"result": "passed", "dpi": 144, "color": "RGB", "all_pages": True, "comparisons": comparisons,
                           "expected_pdf_sha256": sha(expected), "expected_authorship": "Independent pypdf source clone with only the requested before-to-after text edit; original raw images retained",
                           "pillow": pillow_version, "poppler": (version.stdout+version.stderr).strip(), "renderer_sha256": sha(tool.read_bytes()),
                           "acknowledged_renderer_diagnostics": sorted(diagnostics), "renders": records}


def generate_fixture():
    source = fixture_bytes()
    source_checks(source)
    require(not oracle.WARNINGS and not storage.WARNINGS, "source reader warnings")
    SOURCE.write_bytes(source)
    PROVENANCE.write_text(json.dumps({"schema": 1, "license": "Apache-2.0", "origin": __doc__.strip(), "external_assets": [],
        "path": "fixtures/lossless-images.pdf", "bytes": len(source), "sha256": sha(source), "generator": "lossless_image_oracle.py --generate-fixture",
        "generator_sha256": sha(Path(__file__).read_bytes()), "oracle_sha256": sha(Path(__file__).read_bytes()),
        "python": platform.python_version(), "pypdf": pypdf.__version__, "zlib": zlib.ZLIB_VERSION, "zlib_runtime": zlib.ZLIB_RUNTIME_VERSION,
        "page_objects": [3, 4], "optimized_images": [7, 8], "unchanged_mask": 9, "nested_shared_use": 10, "tiny_original_winner": 13,
        "sample_sha256": {"7": sha(samples(3)), "8": sha(samples(1)), "9": sha(mask_samples()), "13": sha(TINY)},
        "request": {"required_edit": {"page_index": 0, "old_text": "before", "replacement": "after"}, "predictor": 12},
        "assurance": "Frozen original source identity/reader gate; independent self-check and actual engine evidence are separate."}, indent=2)+"\n", encoding="utf-8")
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
    require(sha(source) == provenance["sha256"] and source == fixture_bytes(), "frozen image source mismatch")
    require(sha(Path(__file__).read_bytes()) == provenance["oracle_sha256"], "image oracle changed; review separate provenance hash")
    require(not oracle.WARNINGS and not storage.WARNINGS, "warnings before scoped gate")
    evidence = ROOT/"evidence"
    paths = {name: evidence/f"lossless-images-{name}.pdf" for name in ["edited-baseline", "incremental", "compact"]}
    baseline = paths["edited-baseline"].read_bytes()
    phases, outputs = [], []
    previous = flatten.checkpoint()
    baseline_checks = verify_baseline(source, baseline)
    phases.append(flatten.warning_phase("source_and_baseline", previous))
    for name in ["incremental", "compact"]:
        previous = flatten.checkpoint()
        candidate = paths[name].read_bytes()
        checks = verify(baseline, candidate)
        if name == "compact":
            require(len(candidate) < len(baseline), "fixture compact final output is not smaller than required edited baseline")
        else:
            require(candidate.startswith(source), "incremental source prefix changed")
        outputs.append({"mode": name, "bytes": len(candidate), "sha256": sha(candidate), "checks": checks})
        phases.append(flatten.warning_phase(name, previous, source, candidate))
    previous = flatten.checkpoint()
    controls, negatives = negative_controls(baseline, paths["compact"].read_bytes())
    phases.append(flatten.warning_phase("corruption_controls", previous))
    report = {"schema": 1, "result": "passed", "scope": __doc__.strip(), "python": platform.python_version(), "pypdf": pypdf.__version__,
              "source_sha256": sha(source), "edited_baseline_sha256": sha(baseline), "baseline_checks": baseline_checks,
              "oracle_sha256": sha(Path(__file__).read_bytes()), "provenance_sha256": sha(PROVENANCE.read_bytes()),
              "example_sha256": sha((REPO/"crates/binas-pdf/examples/preservation_lossless_images.rs").read_bytes()),
              "helper_sha256": {name: sha((ROOT/name).read_bytes()) for name in ["text_field_oracle.py", "flatten_oracle.py", "flate_oracle.py", "geometry_oracle.py", "oracle.py", "requirements.txt", "LICENSE"]},
              "outputs": outputs, "negative_controls": controls, "warning_gate_controls": flatten.warning_gate_controls(source, paths["incremental"].read_bytes()),
              "ignored_layout": ["xref offsets and representation", "trailer Size/Prev/XRefStm", "stream Length"], "rendering": {"result": "not_checked"}}
    if args.pdftoppm:
        previous = flatten.checkpoint()
        render_checks(args.pdftoppm, source, paths, negatives, report)
        phases.append(flatten.warning_phase("independent_render_expectation", previous))
    require([warning for phase in phases for warning in phase["warnings"]] == oracle.WARNINGS == storage.WARNINGS, "reader warning escaped phase accounting")
    report["reader_warnings"] = phases
    name = "lossless-images-evidence.json" if args.pdftoppm else "lossless-images-structure.json"
    (evidence/name).write_text(json.dumps(report, indent=2)+"\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "writer_outputs": 2, "corruption_controls": len(controls), "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
