"""Independent original page-geometry/reorder preservation gate, Apache-2.0.

Source page index 1 receives absolute rotation -90 (normalized 270), MediaBox
[-10,-20,320,420] and CropBox [20,30,280,370], then order [2,0,1]. No content,
annotation coordinates, resources or document structures are otherwise changed.
This frozen synthetic case is not broad PDF compatibility or conformance proof.
"""
import argparse
import copy
import hashlib
import io
import json
import platform
import subprocess
from pathlib import Path

from pypdf import PdfReader, PdfWriter, __version__ as pypdf_version
from pypdf.generic import (
    ArrayObject, DictionaryObject, IndirectObject, NameObject, NumberObject,
    RectangleObject, StreamObject,
)

import oracle

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
SOURCE = ROOT / "fixtures/reorder-structures.pdf"
ORDER = [2, 0, 1]
SOURCE_PAGE = 1
PAGE_OBJECT = 5
MEDIA = [-10, -20, 320, 420]
CROP = [20, 30, 280, 370]
REQUESTED_ROTATION = -90
ROTATION = REQUESTED_ROTATION % 360


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def raw_objects(data):
    # Read before asking pypdf for pages: page flattening may materialize inherited
    # attributes in its in-memory page dictionaries, which are not source edits.
    reader = PdfReader(io.BytesIO(data), strict=True)
    references = {(number, generation) for generation, items in reader.xref.items()
                  for number in items if number}
    references.update((number, 0) for number in reader.xref_objStm)
    objects = {reference: reader.get_object(IndirectObject(*reference, reader))
               for reference in sorted(references)}
    return reader, objects


def complete_inventory(data):
    reader, objects = raw_objects(data)
    return {
        "objects": {f"{n}:{g}": oracle.canonical(value) for (n, g), value in objects.items()},
        "encoded_streams": {f"{n}:{g}": sha(value._data)
                            for (n, g), value in objects.items() if isinstance(value, StreamObject)},
        "trailer": {str(key): oracle.canonical(value) for key, value in reader.trailer.items()
                    if key not in {"/Size", "/Prev", "/XRefStm"}},
    }


def expected_complete_inventory(source):
    expected = copy.deepcopy(complete_inventory(source))
    objects = expected["objects"]
    require(objects["2:0"]["/Kids"] == [["ref", 4, 0], ["ref", 5, 0]], "source nested page slots changed")
    require(objects["3:0"]["/Kids"] == [["ref", 6, 0]], "source final page slot changed")
    objects["2:0"]["/Kids"] = [["ref", 6, 0], ["ref", 4, 0]]
    objects["3:0"]["/Kids"] = [["ref", 5, 0]]
    # Preserve the moving page's original effective inherited attributes, rather
    # than taking them from its new parent. Page 4 stays in its original parent.
    for page, old_parent, new_parent in [(5, 2, 3), (6, 3, 2)]:
        leaf = objects[f"{page}:0"]
        leaf["/Parent"] = ["ref", new_parent, 0]
        for key in ["/Resources", "/MediaBox", "/CropBox", "/Rotate"]:
            if key not in leaf or leaf[key] is None:
                leaf[key] = copy.deepcopy(objects[f"{old_parent}:0"][key])
    objects["5:0"].update({"/Rotate": ROTATION, "/MediaBox": MEDIA, "/CropBox": CROP})
    source_labels = objects["23:0"]["/Nums"]
    require(source_labels[::2] == [0, 1, 2], "source explicit labels changed")
    objects["23:0"]["/Nums"] = [item for index, original in enumerate(ORDER)
                                  for item in [index, copy.deepcopy(source_labels[original * 2 + 1])]]
    return expected


def verify(source, candidate):
    before = oracle.inventory(source)
    after = oracle.inventory(candidate)
    expected_pages = copy.deepcopy([before["pages"][index] for index in ORDER])
    target = expected_pages[ORDER.index(SOURCE_PAGE)]
    require(target["id"] == PAGE_OBJECT, "frozen source page identity changed")
    target.update(media=MEDIA, crop=CROP, rotate=ROTATION)
    require(after["pages"] == expected_pages,
            "requested page geometry/order or unrelated page text/resources/annotations/tags changed")
    require(after["labels"] == [before["labels"][index] for index in ORDER], "page labels did not follow source identities")
    for key in ["objects", "info", "trailer_id", "fields", "attachments"]:
        require(after[key] == before[key], f"unrequested preservation change: {key}")
    require(after["named_destinations"] == {
        name: ORDER.index(position) for name, position in before["named_destinations"].items()
    }, "named destinations do not follow stable page identities")
    require(after["outline_destinations"] == [ORDER.index(index) for index in before["outline_destinations"]],
            "outline destinations do not follow stable page identities")
    expected = expected_complete_inventory(source)
    actual = complete_inventory(candidate)
    require(set(actual["objects"]) == set(expected["objects"]), "object identities added or removed")
    for identity, value in expected["objects"].items():
        require(actual["objects"][identity] == value, f"unexpected exact object change: {identity}")
    require(actual["encoded_streams"] == expected["encoded_streams"], "geometry operation changed encoded stream bytes")
    require(actual["trailer"] == expected["trailer"], "meaningful trailer changed")
    return {"semantic_inventory": after, "complete_inventory": actual}


def rewrite_objects(data, mutation):
    """Generate a reparsable negative control without changing object identities."""
    reader, objects = raw_objects(data)
    mutation(objects)
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
    trailer = DictionaryObject({key: value for key, value in reader.trailer.items()
                                if key not in {"/Size", "/Prev", "/XRefStm"}})
    trailer[NameObject("/Size")] = NumberObject(size)
    trailer.write_to_stream(output)
    output.write(f"\nstartxref\n{xref}\n%%EOF\n".encode())
    return output.getvalue()


def negative_controls(source, candidate):
    source_page = oracle.inventory(source)["pages"][SOURCE_PAGE]

    def geometry(page, media, crop, rotation):
        page[NameObject("/MediaBox")] = RectangleObject(media)
        page[NameObject("/CropBox")] = RectangleObject(crop)
        page[NameObject("/Rotate")] = NumberObject(rotation)

    def wrong_page(objects):
        geometry(objects[(4, 0)], MEDIA, CROP, ROTATION)
        geometry(objects[(5, 0)], source_page["media"], source_page["crop"], source_page["rotate"])

    def additive_rotation(objects):
        objects[(5, 0)][NameObject("/Rotate")] = NumberObject(
            (source_page["rotate"] + REQUESTED_ROTATION) % 360)

    def missed_crop(objects):
        objects[(5, 0)][NameObject("/CropBox")] = RectangleObject(source_page["crop"])

    def catalog_loss(objects):
        del objects[(1, 0)][NameObject("/Outlines")]

    results, outputs = {}, {}
    for name, mutate in [("wrong_page", wrong_page), ("additive_rotation", additive_rotation),
                         ("missed_crop", missed_crop), ("catalog_loss", catalog_loss)]:
        corrupted = rewrite_objects(candidate, mutate)
        require(len(PdfReader(io.BytesIO(corrupted), strict=True).pages) == 3,
                f"{name} did not remain independently reparsable")
        try:
            verify(source, corrupted)
        except (RuntimeError, AssertionError) as error:
            results[name] = {"reparse": "passed", "detected": str(error), "sha256": sha(corrupted)}
        else:
            raise RuntimeError(f"oracle accepted {name} corruption")
        outputs[name] = corrupted
    return results, outputs


def independent_expected_pdf(source):
    """Pypdf clones the original document, sets absolute geometry, and reorders.

    This separate visual expectation may use different object numbers/topology.
    It is never the source of expected object inventories in verify().
    """
    writer = PdfWriter(clone_from=PdfReader(io.BytesIO(source), strict=True))
    pages = list(writer.pages)
    # Preserve effective source attributes before moving to a flat page tree.
    # Pypdf page access exposes inherited Resources, MediaBox, CropBox and Rotate.
    for page in pages:
        page[NameObject("/Resources")] = page.raw_get("/Resources")
        page[NameObject("/MediaBox")] = RectangleObject(list(page.mediabox))
        page[NameObject("/CropBox")] = RectangleObject(list(page.cropbox))
        page[NameObject("/Rotate")] = NumberObject(page.get("/Rotate", 0))
    pages[SOURCE_PAGE][NameObject("/Rotate")] = NumberObject(ROTATION)
    pages[SOURCE_PAGE][NameObject("/MediaBox")] = RectangleObject(MEDIA)
    pages[SOURCE_PAGE][NameObject("/CropBox")] = RectangleObject(CROP)
    root_pages = writer.root_object.raw_get("/Pages")
    page_tree = root_pages.get_object()
    page_tree[NameObject("/Kids")] = ArrayObject([pages[index].indirect_reference for index in ORDER])
    page_tree[NameObject("/Count")] = NumberObject(3)
    for page in pages:
        page[NameObject("/Parent")] = root_pages
    labels = writer.root_object["/PageLabels"]
    old_labels = list(labels["/Nums"])
    labels[NameObject("/Nums")] = ArrayObject([
        item for index, original in enumerate(ORDER)
        for item in [NumberObject(index), old_labels[original * 2 + 1]]
    ])
    # Pinned pypdf 6.10.0 page cache follows the explicitly authored page tree.
    writer.flattened_pages = [pages[index] for index in ORDER]
    result = io.BytesIO()
    writer.write(result)
    expected = result.getvalue()
    reopened = PdfReader(io.BytesIO(expected), strict=True)
    require(len(reopened.pages) == 3, "independent expected PDF lost pages")
    target = reopened.pages[ORDER.index(SOURCE_PAGE)]
    require(list(target.mediabox) == MEDIA and list(target.cropbox) == CROP
            and target.rotation == ROTATION, "independent expected PDF has wrong absolute geometry")
    require([page.extract_text() for page in reopened.pages]
            == [oracle.inventory(source)["pages"][index]["text"] for index in ORDER],
            "independent expected PDF changed text")
    return expected


def render_checks(tool, source, paths, additive_control, report):
    from PIL import Image, ImageChops, __version__ as pillow_version
    tool = tool.resolve(strict=True)
    work = REPO / "target/binas-validation/geometry"
    work.mkdir(parents=True, exist_ok=True)
    expected = independent_expected_pdf(source)
    expected_path = work / "independent-expected.pdf"
    expected_path.write_bytes(expected)
    negative_path = work / "negative-additive-rotation.pdf"
    negative_path.write_bytes(additive_control)
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    images, rendered = {}, []
    for name, path in [("expected", expected_path), *paths.items(), ("negative", negative_path)]:
        images[name] = []
        for page in [1, 2, 3]:
            prefix = work / f"{name}-{page}"
            command = [str(tool), "-f", str(page), "-l", str(page), "-singlefile", "-r", "144",
                       "-cropbox", "-png", str(path), str(prefix)]
            result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
            png = Path(str(prefix) + ".png")
            with Image.open(png) as opened:
                image = opened.convert("RGB")
            images[name].append(image)
            rendered.append({"name": name, "page": page, "command": command, "stderr": result.stderr,
                             "size": list(image.size), "rgb_sha256": sha(image.tobytes()), "png_sha256": sha(png.read_bytes())})

    def same(left, right):
        return left.size == right.size and ImageChops.difference(left, right).getbbox() is None

    comparisons = {}
    for name in paths:
        comparisons[name] = [same(left, right) for left, right in zip(images["expected"], images[name])]
        require(comparisons[name] == [True, True, True], f"{name} differs from independent RGB144 expected rendering")
    negative = [same(left, right) for left, right in zip(images["expected"], images["negative"])]
    require(negative == [True, True, False], "additive rotation was not detected only on the requested source page")
    report["rendering"] = {
        "result": "passed", "color": "RGB", "dpi": 144, "all_pages": True,
        "expected_pdf_sha256": sha(expected), "expected_authorship": "pypdf source clone, absolute geometry assignment, explicit reorder",
        "pillow": pillow_version, "poppler": (version.stdout + version.stderr).strip(),
        "renderer_sha256": sha(tool.read_bytes()), "comparisons": comparisons,
        "additive_rotation_control_equal": negative, "renders": rendered,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--structure-only", action="store_true")
    mode.add_argument("--pdftoppm", type=Path)
    args = parser.parse_args()
    require(pypdf_version == "6.10.0", "install pinned pypdf 6.10.0")
    source = SOURCE.read_bytes()
    manifest = json.loads((ROOT / "manifest.json").read_text(encoding="utf-8"))
    entry = next(item for item in manifest["required"] if item["path"] == "fixtures/reorder-structures.pdf")
    require(sha(source) == entry["sha256"], "frozen geometry source hash mismatch")
    oracle.source_checks(source)
    paths = {name: ROOT / f"evidence/geometry-{name}.pdf" for name in ["incremental", "compact"]}
    # Required candidates are read unconditionally: missing artifacts fail this gate.
    candidates = {name: path.read_bytes() for name, path in paths.items()}
    outputs = []
    for name, candidate in candidates.items():
        if name == "incremental":
            require(candidate.startswith(source), "incremental output lost its exact input prefix")
        outputs.append({"mode": name, "bytes": len(candidate), "sha256": sha(candidate),
                        "checks": verify(source, candidate)})
    controls, negative_bytes = negative_controls(source, candidates["compact"])
    report = {
        "schema": 1, "result": "passed", "scope": __doc__.strip(),
        "python": platform.python_version(), "pypdf": pypdf_version,
        "source_sha256": sha(source), "oracle_sha256": sha(Path(__file__).read_bytes()),
        "example_sha256": sha((REPO / "crates/binas-pdf/examples/preservation_geometry.rs").read_bytes()),
        "sha256": {name: sha((ROOT / name).read_bytes()) for name in ["oracle.py", "manifest.json", "requirements.txt", "LICENSE"]},
        "request": {"source_page_index": SOURCE_PAGE, "stable_page_object": PAGE_OBJECT,
                    "absolute_rotation": REQUESTED_ROTATION, "normalized_rotation": ROTATION,
                    "media_box": MEDIA, "crop_box": CROP, "order": ORDER},
        "outputs": outputs, "negative_controls": controls, "rendering": {"result": "not_checked"},
        "ignored_layout": ["stream Length representation", "xref offsets and representation", "trailer Size/Prev/XRefStm"],
    }
    if args.pdftoppm:
        render_checks(args.pdftoppm, source, paths, negative_bytes["additive_rotation"], report)
    report["reader_warnings"] = sorted(set(oracle.WARNINGS))
    receipt = "geometry-evidence.json" if args.pdftoppm else "geometry-structure.json"
    (ROOT / "evidence" / receipt).write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "writer_outputs": 2, "corruption_controls": len(controls),
                      "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
