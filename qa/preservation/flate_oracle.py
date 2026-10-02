"""Original Apache-2.0 Flate storage fixture and independent pypdf/Poppler gate.

This checks one synthetic Flate-to-Flate pass after an edit. It is not a general
optimization benchmark, a font-fidelity claim, or a conformance validator.
"""
import argparse
import hashlib
import io
import json
import logging
import platform
import subprocess
import zlib
from pathlib import Path

import pypdf
from pypdf.generic import (
    ArrayObject, BooleanObject, ByteStringObject, DictionaryObject,
    EncodedStreamObject, FloatObject, IndirectObject, NameObject, NullObject,
    NumberObject, StreamObject, TextStringObject,
)

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
FIXTURE = ROOT / "fixtures/flate-storage.pdf"
PROVENANCE = ROOT / "fixtures/flate-storage.provenance.json"
WARNINGS = []


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


class CaptureWarnings(logging.Handler):
    def emit(self, record):
        if record.levelno >= logging.WARNING:
            WARNINGS.append(record.getMessage())


logging.getLogger("pypdf").addHandler(CaptureWarnings())


def stream(data, extra=b"", weak_flate=False):
    if weak_flate:
        data = zlib.compress(data, level=0)
        extra += b" /Filter /FlateDecode"
    return (b"<< /Length " + str(len(data)).encode() + b" " + extra
            + b" >>\nstream\n" + data + b"\nendstream")


def serialize_raw(objects):
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for number, value in sorted(objects.items()):
        offsets[number] = len(output)
        output += f"{number} 0 obj\n".encode() + value + b"\nendobj\n"
    xref = len(output)
    size = max(objects) + 1
    output += f"xref\n0 {size}\n0000000000 65535 f \n".encode()
    for number in range(1, size):
        output += (f"{offsets[number]:010} 00000 n \n" if number in offsets
                   else "0000000000 00000 f \n").encode()
    output += (f"trailer\n<< /Size {size} /Root 1 0 R /Info 16 0 R "
               "/ID [<00112233445566778899AABBCCDDEEFF> "
               "<00112233445566778899AABBCCDDEEFF>] >>\n"
               f"startxref\n{xref}\n%%EOF\n").encode()
    return bytes(output)


def fixture_bytes():
    padding = b"% Original repeated storage padding; no drawing or cleanup request.\n" * 160
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R /Metadata 10 0 R /Outlines 11 0 R /Names 13 0 R /Lang (en-US) >>",
        2: b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 240 200] /Resources << /Font << /F1 8 0 R >> /XObject << /Shared 7 0 R >> >> >>",
        3: b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
        4: b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>",
        5: stream(b"BT /F1 18 Tf 30 160 Td (before) Tj ET\nq 1 0 0 1 30 80 cm /Shared Do Q\n" + padding, weak_flate=True),
        6: stream(b"BT /F1 18 Tf 30 160 Td (untouched) Tj ET\nq 1 0 0 1 90 80 cm /Shared Do Q\n" + padding, weak_flate=True),
        7: stream(b"q 0.2 0.4 0.8 rg 0 0 80 30 re f Q\nBT /F1 10 Tf 5 10 Td (shared) Tj ET\n" + padding,
                  b"/Type /XObject /Subtype /Form /BBox [0 0 80 30] /Resources << /Font << /F1 8 0 R >> >>", True),
        8: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 9 0 R >>",
        9: stream(b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /OriginalASCII def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfrange <20> <7E> <0020> endbfrange endcmap CMapName currentdict /CMap defineresource pop end end"),
        10: stream(b'<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Original Flate storage fixture</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>', b"/Type /Metadata /Subtype /XML"),
        11: b"<< /Type /Outlines /First 12 0 R /Last 12 0 R /Count 1 >>",
        12: b"<< /Title (Second page) /Parent 11 0 R /Dest [4 0 R /Fit] >>",
        13: b"<< /Dests << /Names [(next) [4 0 R /Fit]] >> /EmbeddedFiles << /Names [(note.txt) 14 0 R] >> >>",
        14: b"<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F 15 0 R >> /Desc (Exact original attachment) >>",
        15: stream(b"Original attachment: retain all bytes, including NUL:\x00\n", b"/Type /EmbeddedFile /Subtype /text#2Fplain"),
        16: b"<< /Title (Original Flate storage fixture) /Author (Binas original Apache-2.0 fixture) >>",
    }
    return serialize_raw(objects)


def generate_fixture():
    data = fixture_bytes()
    FIXTURE.write_bytes(data)
    provenance = {
        "schema": 1, "license": "Apache-2.0", "origin": __doc__.strip(),
        "external_assets": [], "path": str(FIXTURE.relative_to(ROOT)).replace("\\", "/"),
        "sha256": sha(data), "bytes": len(data),
        "generator": "flate_oracle.py --generate-fixture",
        "generator_sha256": sha(Path(__file__).read_bytes()),
        "python": platform.python_version(), "zlib": zlib.ZLIB_VERSION,
        "zlib_runtime": zlib.ZLIB_RUNTIME_VERSION,
        "page_objects": [3, 4], "page_streams": [5, 6], "shared_form": 7,
        "weak_flate_level": 0, "requested_edit": {"page": 0, "old": "before", "new": "after"},
        "retained_assets": {"cmap": 9, "metadata": 10, "outline": 12, "names": 13, "attachment": 15},
        "assurance": "Synthetic storage regression only. Reader and optional RGB evidence require a separate successful run.",
    }
    PROVENANCE.write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"fixture_sha256": sha(data), "bytes": len(data)}))


def typed(value):
    if isinstance(value, IndirectObject):
        return ["ref", value.idnum, value.generation]
    if isinstance(value, StreamObject):
        return ["stream", stream_dictionary(value), sha(value.get_data())]
    if isinstance(value, DictionaryObject):
        return ["dict", [[str(key), typed(item)] for key, item in sorted(value.items())]]
    if isinstance(value, ArrayObject):
        return ["array", [typed(item) for item in value]]
    if isinstance(value, NameObject):
        return ["name", str(value)]
    if isinstance(value, TextStringObject):
        return ["text", str(value), value.original_bytes.hex()]
    if isinstance(value, ByteStringObject):
        return ["bytes", bytes(value).hex()]
    if isinstance(value, BooleanObject):
        return ["bool", value.value]
    if isinstance(value, NullObject):
        return ["null"]
    if isinstance(value, FloatObject):
        return ["real", float(value).hex()]
    if isinstance(value, NumberObject):
        return ["integer", int(value)]
    raise RuntimeError(f"unhandled oracle value type: {type(value).__name__}")


def stream_dictionary(value):
    return [[str(key), typed(item)] for key, item in sorted(value.items()) if key != "/Length"]


def inventory(data):
    reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
    references = {(number, generation) for generation, items in reader.xref.items()
                  for number in items if number}
    references.update((number, 0) for number in reader.xref_objStm)
    return reader, {reference: reader.get_object(IndirectObject(*reference, reader))
                    for reference in sorted(references)}


def trailer(reader):
    return {str(key): typed(value) for key, value in reader.trailer.items()
            if key not in {"/Size", "/Prev", "/XRefStm"}}


def compare(before_bytes, after_bytes, requested_edit=False, require_recompression=False):
    before, old = inventory(before_bytes)
    after, new = inventory(after_bytes)
    require(set(old) == set(new), "object identities added or removed")
    require(trailer(before) == trailer(after), "meaningful trailer changed")
    require([p.indirect_reference.idnum for p in before.pages] == [3, 4], "unexpected baseline page IDs")
    require([p.indirect_reference.idnum for p in after.pages] == [3, 4], "page mapping changed")
    encoded_changes = []
    for reference, original in old.items():
        candidate = new[reference]
        label = f"{reference[0]}:{reference[1]}"
        if isinstance(original, StreamObject):
            require(isinstance(candidate, StreamObject), f"stream {label} became another type")
            require(stream_dictionary(original) == stream_dictionary(candidate), f"stream dictionary {label} changed")
            expected = original.get_data()
            if requested_edit and reference == (5, 0):
                require(expected.count(b"(before)") == 1, "edit control token changed")
                expected = expected.replace(b"(before)", b"(after)")
            require(expected == candidate.get_data(), f"decoded stream {label} changed")
            # Pinned pypdf 6.10.0 encoded payload access is confined to this oracle.
            if original._data != candidate._data:
                require(reference in {(5, 0), (6, 0), (7, 0)}, f"unrequested encoded asset change {label}")
                require(candidate.get("/Filter") == "/FlateDecode", f"filter changed {label}")
                require(zlib.decompress(candidate._data) == expected, f"invalid Flate {label}")
                encoded_changes.append({"stream": label, "before": len(original._data),
                                        "after": len(candidate._data), "decoded_sha256": sha(expected)})
        else:
            require(typed(original) == typed(candidate), f"typed object {label} changed")
    old_text = [page.extract_text() for page in before.pages]
    new_text = [page.extract_text() for page in after.pages]
    if requested_edit:
        require("before" in old_text[0] and "after" in new_text[0] and "before" not in new_text[0], "requested text edit absent")
        require(old_text[0].replace("before", "after") == new_text[0], "unrequested extracted text change")
        require(old_text[1] == new_text[1], "untouched page text changed")
    else:
        require(old_text == new_text, "extracted text changed during storage-only pass")
    require(after.attachments["note.txt"] == before.attachments["note.txt"], "attachment bytes changed")
    if require_recompression:
        require(any(item["stream"] == "7:0" for item in encoded_changes), "shared Form was not recompressed")
        require(all(item["after"] < item["before"] for item in encoded_changes), "encoded change was not smaller")
        require(len(after_bytes) < len(before_bytes), "fixture final artifact was not smaller than edited baseline")
    return {"object_count": len(new), "page_ids": [3, 4], "text": new_text,
            "encoded_changes": encoded_changes, "final_before": len(before_bytes),
            "final_after": len(after_bytes), "final_bytes_saved": len(before_bytes) - len(after_bytes),
            "objects": {f"{n}:{g}": typed(value) for (n, g), value in new.items()}}


def corrupt_valid_flate(data):
    reader, objects = inventory(data)
    target = objects[(5, 0)]
    decoded = target.get_data()
    require(decoded.count(b"(after)") == 1, "negative control token absent")
    bad_decoded = decoded.replace(b"(after)", b"(wrong)")
    replacement = EncodedStreamObject()
    replacement.update(target)
    replacement._data = zlib.compress(bad_decoded, level=9)
    objects[(5, 0)] = replacement
    output = io.BytesIO()
    output.write(b"%PDF-1.7\n")
    offsets = {}
    for (number, generation), value in objects.items():
        require(generation == 0, "fixture generation changed")
        offsets[number] = output.tell()
        output.write(f"{number} 0 obj\n".encode())
        value.write_to_stream(output)
        output.write(b"\nendobj\n")
    xref = output.tell()
    size = max(offsets) + 1
    output.write(f"xref\n0 {size}\n0000000000 65535 f \n".encode())
    for number in range(1, size):
        output.write((f"{offsets[number]:010} 00000 n \n" if number in offsets
                      else "0000000000 00000 f \n").encode())
    output.write(b"trailer\n")
    rebuilt_trailer = DictionaryObject({key: value for key, value in reader.trailer.items()
                                       if key not in {"/Size", "/Prev", "/XRefStm"}})
    rebuilt_trailer[NameObject("/Size")] = NumberObject(size)
    rebuilt_trailer.write_to_stream(output)
    output.write(f"\nstartxref\n{xref}\n%%EOF\n".encode())
    corrupted = output.getvalue()
    _, reopened = inventory(corrupted)
    require(reopened[(5, 0)].get_data() == bad_decoded, "negative control did not reparse as valid altered Flate")
    return corrupted


def render_compare(tool, baseline, candidate, negative, report):
    from PIL import Image, ImageChops, __version__ as pillow_version
    tool = tool.resolve(strict=True)
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    work = REPO / "target/binas-validation/flate"
    work.mkdir(parents=True, exist_ok=True)
    negative_path = work / "valid-flate-text-corruption.pdf"
    negative_path.write_bytes(negative)
    renders = {}
    entries = []
    for name, path in [("edited", baseline), ("compressed", candidate), ("negative", negative_path)]:
        renders[name] = []
        for page in [1, 2]:
            prefix = work / f"{name}-{page}"
            command = [str(tool), "-f", str(page), "-l", str(page), "-singlefile", "-r", "144", "-cropbox", "-png", str(path), str(prefix)]
            result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
            png = Path(str(prefix) + ".png")
            with Image.open(png) as opened:
                image = opened.convert("RGB")
            renders[name].append(image)
            entries.append({"name": name, "page": page, "command": command, "stderr": result.stderr,
                            "size": list(image.size), "rgb_sha256": sha(image.tobytes()), "png_sha256": sha(png.read_bytes())})
    def same(a, b):
        return a.size == b.size and ImageChops.difference(a, b).getbbox() is None
    matches = [same(a, b) for a, b in zip(renders["edited"], renders["compressed"])]
    control = [same(a, b) for a, b in zip(renders["edited"], renders["negative"])]
    require(matches == [True, True], "recompression changed RGB144dpi rendering")
    require(control == [False, True], "valid-Flate text corruption was not detected only on its affected page")
    report["rendering"] = {"result": "passed", "dpi": 144, "color": "RGB", "all_pages": True,
                           "baseline_equals_candidate": matches, "baseline_equals_negative": control,
                           "poppler": (version.stdout + version.stderr).strip(), "pillow": pillow_version,
                           "renderer_sha256": sha(tool.read_bytes()), "renders": entries}


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
    provenance = json.loads(PROVENANCE.read_text(encoding="utf-8"))
    source = FIXTURE.read_bytes()
    require(sha(source) == provenance["sha256"], "frozen fixture hash mismatch")
    require(sha(Path(__file__).read_bytes()) == provenance["generator_sha256"], "fixture generator/oracle changed; explicitly regenerate and review provenance")
    evidence = ROOT / "evidence"
    pristine = (evidence / "flate-pristine.pdf").read_bytes()
    baseline_path = evidence / "flate-edited-baseline.pdf"
    candidate_path = evidence / "flate-compressed.pdf"
    baseline, candidate = baseline_path.read_bytes(), candidate_path.read_bytes()
    compare(source, pristine)
    requested = compare(source, baseline, requested_edit=True)
    compressed = compare(baseline, candidate, require_recompression=True)
    negative = corrupt_valid_flate(candidate)
    try:
        compare(baseline, negative, require_recompression=True)
    except RuntimeError as error:
        failure = str(error)
        require(failure == "decoded stream 5:0 changed", f"control failed for an unrelated reason: {failure}")
    else:
        raise RuntimeError("valid-Flate corruption escaped independent checks")
    report = {
        "schema": 1, "result": "passed", "scope": __doc__.strip(),
        "pypdf": pypdf.__version__, "python": platform.python_version(),
        "source_sha256": sha(source), "pristine_sha256": sha(pristine),
        "edited_baseline_sha256": sha(baseline), "compressed_sha256": sha(candidate),
        "provenance_sha256": sha(PROVENANCE.read_bytes()), "oracle_sha256": sha(Path(__file__).read_bytes()),
        "example_sha256": sha((REPO / "crates/binas-pdf/examples/preservation_flate.rs").read_bytes()),
        "ignored_layout": ["stream /Length", "xref offsets and representation", "trailer /Size /Prev /XRefStm"],
        "requested_edit": requested, "storage_pass": compressed,
        "negative_control": {"sha256": sha(negative), "valid_flate_reparse": True, "detected": failure},
        "rendering": {"result": "not_checked"},
    }
    if args.pdftoppm:
        render_compare(args.pdftoppm, baseline_path, candidate_path, negative, report)
    report["reader_warnings"] = WARNINGS
    require(not WARNINGS, "independent reader emitted warnings")
    name = "flate-evidence.json" if args.pdftoppm else "flate-structure.json"
    (evidence / name).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "final_bytes_saved": compressed["final_bytes_saved"],
                      "encoded_streams_changed": len(compressed["encoded_changes"]),
                      "negative_control_detected": True, "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
