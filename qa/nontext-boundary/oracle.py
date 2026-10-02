"""Independent pypdf + Poppler gate for public lifecycle and Contents controls.
Original Binas QA, Apache-2.0. Reuses the frozen text-field fixture; no new corpus.
"""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import sys

import pypdf
from pypdf.generic import StreamObject, NameObject, TextStringObject
from PIL import Image, ImageChops

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "qa/preservation"))
import flate_oracle as storage
import flatten_oracle as flatten
import oracle as preservation


def require(condition, reason):
    if not condition:
        raise AssertionError(reason)


def same_except(a, b, keys):
    require({k: storage.typed(v) for k, v in a.items() if k not in keys} ==
            {k: storage.typed(v) for k, v in b.items() if k not in keys},
            f"unexpected dictionary change outside {keys}")


def verify(before, after, changes=None, changed_streams=None):
    changes = changes or {}
    changed_streams = changed_streams or set()
    first, old = storage.inventory(before)
    reader, new = storage.inventory(after)
    require(set(old).issubset(new), "original object removed")
    require(storage.trailer(first) == storage.trailer(reader), "meaningful trailer changed")
    require([p.indirect_reference.idnum for p in first.pages] ==
            [p.indirect_reference.idnum for p in reader.pages], "page order/identity changed")
    for ref, value in old.items():
        if ref in changes:
            same_except(value, new[ref], changes[ref])
        else:
            require(storage.typed(value) == storage.typed(new[ref]), f"unrequested object change {ref}")
        if isinstance(value, StreamObject) and ref not in changed_streams:
            require(new[ref]._data == value._data, f"original encoded stream changed {ref}")
    return {"original_objects": len(old), "new_objects": len(set(new) - set(old)),
            "untouched_original_encoded_streams_exact": True,
            "declared_changed_streams": sorted(changed_streams)}


def main():
    if not __debug__ or os.environ.get("PYTHONOPTIMIZE"):
        raise RuntimeError("Independent oracle requires unoptimized Python")
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--pdftoppm", type=Path, required=True)
    args = p.parse_args()
    out = args.output.resolve()
    source_path = REPO / "qa/preservation/fixtures/text-field-fill.pdf"
    source = source_path.read_bytes()
    provenance = json.loads(source_path.with_suffix(".provenance.json").read_text())
    require(storage.sha(source) == provenance["sha256"], "source identity mismatch")
    require(pypdf.__version__ == "6.10.0", "pinned pypdf version required")
    source_reader, old = storage.inventory(source)
    page = (source_reader.pages[0].indirect_reference.idnum, 0)
    acro_ref = source_reader.trailer["/Root"].raw_get("/AcroForm")
    acro = (acro_ref.idnum, acro_ref.generation)
    records, phases = [], []

    def check(name, baseline, changes=None):
        raw = (out / f"{name}.pdf").read_bytes()
        previous = flatten.checkpoint()
        checks = verify(baseline, raw, changes)
        phase = "incremental" if name in {"annotation-legacy", "annotation-incremental"} else name
        phases.append(flatten.warning_phase(phase, previous, baseline, raw) | {"artifact": name})
        records.append({"name": name, "bytes": len(raw), "sha256": storage.sha(raw), "checks": checks})
        return raw

    created = check("annotation-created", source, {page: {"/Annots"}})
    _, added = storage.inventory(created)
    annotations = added[page]["/Annots"]
    require([storage.typed(a) for a in annotations[:-1]] == [storage.typed(a) for a in old[page]["/Annots"]], "old annotation registration changed")
    note = annotations[-1].get_object()
    require(note["/Subtype"] == "/Text" and str(note["/Contents"]) == "boundary note", "created note effect absent")
    require(note["/AP"]["/N"].get_data(), "created note appearance missing")
    require(list(note["/Rect"]) == [40, 40, 64, 64], "created note geometry changed")
    annotation_removed = check("annotation-removed", source)
    seed = (out / "annotation-seed.pdf").read_bytes()
    old_body = b"<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [30 35 200 55] /Dest [4 0 R /Fit] /Border [0 0 1] /Contents (Retained link) >>"
    new_body = b"<< /Type /Annot /Subtype /Text /P 3 0 R /Rect [30 35 200 55] /Contents (boundary note) >>"
    expected_seed = source.replace(old_body, new_body.ljust(len(old_body), b" "))
    require(seed == expected_seed and len(seed) == len(source), "undeclared derived shape control")
    verify(source, seed, {(18, 0): {"/Subtype", "/Contents", "/Dest", "/Border"}})
    candidates = {}
    for name in ["annotation-legacy", "annotation-incremental", "annotation-compact"]:
        raw = check(name, seed, {(18, 0): {"/Contents"}})
        reader, objs = storage.inventory(raw)
        require(str(objs[(18, 0)]["/Contents"]) == "boundary changed é", "Contents effect absent")
        require("/AP" not in objs[(18, 0)], "appearance-free note unexpectedly generated AP")
        candidates[name] = raw
    form = check("form-created", source, {page: {"/Annots"}, acro: {"/Fields", "/NeedAppearances"}})
    _, forms = storage.inventory(form)
    require([storage.typed(a) for a in forms[acro]["/Fields"][:-1]] ==
            [storage.typed(a) for a in old[acro]["/Fields"]], "old field registration changed")
    field = forms[acro]["/Fields"][-1].get_object()
    require(field["/T"] == "boundary_field" and field["/FT"] == "/Tx" and field["/V"] == "initial", "created field effect absent")
    require(len(field["/Kids"]) == 1, "created field widget count differs")
    widget_ref = field["/Kids"][0]
    widget = widget_ref.get_object()
    require(storage.typed(forms[page]["/Annots"][-1]) == storage.typed(widget_ref), "created widget not page-registered")
    require([storage.typed(a) for a in forms[page]["/Annots"][:-1]] == [storage.typed(a) for a in old[page]["/Annots"]], "old form widgets changed")
    require(widget["/AP"]["/N"].get_data(), "created field appearance absent")
    require(list(widget["/Rect"]) == [40, 80, 180, 104], "created field geometry changed")
    form_removed = check("form-removed", source, {acro: {"/NeedAppearances"}})
    _, removed = storage.inventory(form_removed)
    require(storage.typed(removed[acro]["/Fields"]) == storage.typed(old[acro]["/Fields"]), "removed field still active")
    negatives = []
    # A preservation-only comparison can pass the unchanged input; requested
    # registration effects must independently reject both missing-targets.
    for name, target_exists in [
        ("missing-created-annotation", len(old[page]["/Annots"]) == len(annotations)),
        ("missing-created-form", any(str(a.get_object().get("/T")) == "boundary_field" for a in old[acro]["/Fields"])),
    ]:
        require(not target_exists, f"missing target control escaped: {name}")
        negatives.append({"name": name, "reparse": "passed", "detected": True,
                          "reason": "Unchanged original lacks requested new registration", "sha256": storage.sha(source)})
    target = candidates["annotation-compact"]
    for name, before, after in [
        ("wrong-contents", "boundary changed é".encode("utf-16-be").hex().upper().encode(),
         "boundary changed ñ".encode("utf-16-be").hex().upper().encode()),
        ("changed-sibling-value", b"/V <4F6C64>", b"/V <426164>"),
        ("changed-unselected-field-limit", b"/MaxLen 24", b"/MaxLen 25"),
    ]:
        require(before in target, f"negative control token absent: {name}")
        corrupt = target.replace(before, after)
        require(len(corrupt) == len(target), "negative length changed")
        storage.inventory(corrupt)  # Must be independently reparseable.
        try:
            verify(seed, corrupt, {(18, 0): {"/Contents"}})
            _, objects = storage.inventory(corrupt)
            require(str(objects[(18, 0)]["/Contents"]) == "boundary changed é", "wrong requested Contents")
        except AssertionError as error:
            negatives.append({"name": name, "reparse": "passed", "detected": True, "reason": str(error), "sha256": storage.sha(corrupt)})
        else:
            raise AssertionError(f"negative escaped: {name}")
    image_source = (REPO / "qa/preservation/fixtures/lossless-images.pdf").read_bytes()
    image_expected_samples = bytes([20, 80, 180]) * 4

    def image_checks(raw):
        checks = verify(image_source, raw, {(13, 0): {"/Filter", "/DecodeParms", "/Length"}}, {(13, 0)})
        _, objects = storage.inventory(raw)
        image = objects[(13, 0)]
        require(image.get_data() == image_expected_samples, "requested exact RGB samples absent")
        require("/Filter" not in image and "/DecodeParms" not in image, "replacement raw storage absent")
        require(image._data == image_expected_samples, "replacement encoded raw bytes differ")
        return checks

    image_raw = (out / "image-replaced.pdf").read_bytes()
    image_result = image_checks(image_raw)
    records.append({"name": "image-replaced", "sha256": storage.sha(image_raw), "bytes": len(image_raw), "checks": image_result})
    require(image_expected_samples in image_raw, "image control samples missing")
    for name, raw in [
        ("missing-image-effect", image_source),
        ("wrong-image-sample", image_raw.replace(image_expected_samples, bytes([21, 80, 180]) + image_expected_samples[3:])),
        ("image-collateral-language", image_raw.replace(b"/Lang <656E2D5553>", b"/Lang <66722D4652>")),
    ]:
        require(name == "missing-image-effect" or raw != image_raw, "image negative token absent")
        storage.inventory(raw)
        try:
            image_checks(raw)
        except AssertionError as error:
            negatives.append({"name": name, "reparse": "passed", "detected": True, "reason": str(error), "sha256": storage.sha(raw)})
        else:
            raise AssertionError(f"image negative escaped: {name}")
    # Independently author the requested Contents with pypdf; glyph/image
    # appearance and removal checks compare all pages, RGB144/CropBox.
    writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(seed), strict=True))
    for annot in writer.pages[0]["/Annots"]:
        obj = annot.get_object()
        if obj.get("/Subtype") == "/Text":
            obj[NameObject("/Contents")] = TextStringObject("boundary changed é")
    buffer = io.BytesIO()
    writer.write(buffer)
    expected = out / "annotation-independent-expected.pdf"
    expected.write_bytes(buffer.getvalue())
    source_render = out / "source.pdf"
    source_render.write_bytes(source)
    image_original_path = out / "image-source.pdf"
    image_original_path.write_bytes(image_source)
    image_writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(image_source), strict=True))
    image_writer.pages[0]["/Resources"]["/XObject"]["/Tiny"].set_data(image_expected_samples)
    image_expected = out / "image-independent-expected.pdf"
    with image_expected.open("wb") as file:
        image_writer.write(file)
    tool = args.pdftoppm.resolve(strict=True)
    version = subprocess.run([str(tool), "-v"], capture_output=True, text=True, check=True)
    rendering = []

    def render(path, name):
        images = []
        for page_index in [1, 2]:
            prefix = out / f"render-{name}-{page_index}"
            command = [str(tool), "-r", "144", "-cropbox", "-png", "-singlefile",
                       "-f", str(page_index), "-l", str(page_index), str(path), str(prefix)]
            result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=60)
            lines = set(result.stderr.splitlines())
            require(lines <= {"Syntax Error: No display font for 'Symbol'", "Syntax Error: No display font for 'ArialUnicode'"}, "unexpected renderer diagnostic")
            with Image.open(str(prefix) + ".png") as image:
                rgb = image.convert("RGB")
            images.append(rgb)
            rendering.append({"name": name, "page": page_index, "rgb_sha256": storage.sha(rgb.tobytes()), "size": list(rgb.size), "stderr": result.stderr, "command": command})
        return images

    references = {"source": render(source_render, "source"), "annotation": render(expected, "expected")}
    comparisons = []
    for name, bounds in [("annotation-created", [40, 40, 64, 64]), ("form-created", [40, 80, 180, 104])]:
        images = render(out / f"{name}.pdf", name)
        for page_index, (actual, baseline) in enumerate(zip(images, references["source"])):
            require(actual.size == baseline.size, "creation changed page size")
            bbox = ImageChops.difference(actual, baseline).getbbox()
            if page_index == 0:
                require(bbox is not None, "created appearance has no visible effect")
                height = float(source_reader.pages[0].cropbox.top)
                permitted = [bounds[0] * 2 - 2, (height - bounds[3]) * 2 - 2,
                             bounds[2] * 2 + 2, (height - bounds[1]) * 2 + 2]
                require(bbox[0] >= permitted[0] and bbox[1] >= permitted[1] and bbox[2] <= permitted[2] and bbox[3] <= permitted[3], "creation changed pixels outside declared rectangle")
                comparisons.append({"name": name, "page": page_index, "visible_effect": True, "changed_bbox": bbox, "permitted_bbox": permitted})
            else:
                require(bbox is None, "creation changed unselected page")
                comparisons.append({"name": name, "page": page_index, "pixel_exact": True})
    for name, ref in [("annotation-removed", "source"), ("form-removed", "source"),
                      ("annotation-legacy", "annotation"), ("annotation-incremental", "annotation"),
                      ("annotation-compact", "annotation")]:
        images = render(out / f"{name}.pdf", name)
        for page_index, (a, b) in enumerate(zip(images, references[ref])):
            require(a.size == b.size and ImageChops.difference(a, b).getbbox() is None, f"pixel mismatch: {name}/{page_index}")
            comparisons.append({"name": name, "page": page_index, "pixel_exact": True})
    image_expected_rgb = render(image_expected, "image-expected")
    image_source_rgb = render(image_original_path, "image-source")
    image_actual_rgb = render(out / "image-replaced.pdf", "image-replaced")
    for page_index, (actual, expected_rgb, original_rgb) in enumerate(zip(image_actual_rgb, image_expected_rgb, image_source_rgb)):
        require(actual.size == expected_rgb.size and ImageChops.difference(actual, expected_rgb).getbbox() is None, "image golden pixels mismatch")
        effect = ImageChops.difference(actual, original_rgb).getbbox()
        require((effect is not None) == (page_index == 0), "image intended page effect or untouched page mismatch")
        comparisons.append({"name": "image-replaced", "page": page_index, "independent_expected_pixel_exact": True,
                            "source_pixels_exact": effect is None, "changed_bbox": effect})
    require(set(preservation.WARNINGS) <= {flatten.SPARSE_WARNING}, "unknown reader diagnostic escaped")
    report = {"schema": 1, "result": "passed", "source_sha256": storage.sha(source),
              "derived_seed_sha256": storage.sha(seed), "derived_seed_shape": "same-size Link18 to appearance-free Text note; all other original objects exact",
              "oracle_sha256": storage.sha(Path(__file__).read_bytes()), "pypdf": pypdf.__version__,
              "outputs": records, "negative_controls": negatives, "reader_phases": phases,
              "renderer": (version.stdout + version.stderr).strip(), "renderer_sha256": storage.sha(tool.read_bytes()),
              "rendering": rendering, "comparisons": comparisons,
              "appearance_limits": "create AP reachable/nonempty and visible changes confined to target rectangle; unselected page exact. Icon/text style not independently golden-verified. Contents without AP and removals all-page exact. Raw RGB replacement golden exact; other codecs/masks/colors unchecked."}
    (out / "oracle.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "outputs": len(records), "negative_controls": len(negatives), "pixel_comparisons": len(comparisons)}))


if __name__ == "__main__":
    main()
