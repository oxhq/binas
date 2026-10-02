"""Original-source independent gate for retained page duplication with forms.

Duplicate source page0 once at position1, with independent group.name_copy and
group.keep_copy fields. Their AP/content/font resources remain shared and exact.
Original destinations retain original page identities; labels follow occurrences
[0,0,1]. This synthetic case is not arbitrary merge, tag or action reconciliation.
"""
import argparse
import io
import json
import platform
import subprocess
import zlib
from pathlib import Path

import pypdf
from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, NameObject, NumberObject, TextStringObject, StreamObject

import flate_oracle as storage
import flatten_oracle as flatten
import geometry_oracle as geometry
import oracle
import text_field_oracle as text_fields

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
SOURCE = ROOT / "fixtures/text-field-fill.pdf"
SOURCE_PROVENANCE = ROOT / "fixtures/text-field-fill.provenance.json"
PROVENANCE = ROOT / "fixtures/page-duplicate.provenance.json"
MAPPING = [0, 0, 1]
RENAMES = {"group.name": "group.name_copy", "group.keep": "group.keep_copy"}
sha, require = storage.sha, storage.require


def source_checks(source):
    result = text_fields.source_checks(source)
    _, objects = storage.inventory(source)
    require([value.idnum for value in objects[(3, 0)]["/Annots"]] == [9, 11, 18], "source annotation order changed")
    require(storage.typed(objects[(18, 0)]["/Dest"]) == ["array", [["ref", 4, 0], ["name", "/Fit"]]], "source explicit destination changed")
    return result


def identity(value):
    return flatten.raw_identity(value)


def roles(objects):
    children = objects[(2, 0)]["/Kids"]
    require(len(children) == 3, "expected one duplicated page occurrence")
    page = identity(children[1])
    fields = objects[(7, 0)]["/Kids"]
    require(len(fields) == 4, "independent sibling fields not appended")
    name_field, keep_field = identity(fields[2]), identity(fields[3])
    widgets = objects[name_field]["/Kids"]
    require(len(widgets) == 1, "copied name field must own only its selected-page widget")
    name_widget = identity(widgets[0])
    annots = objects[page]["/Annots"]
    require(len(annots) == 3, "copied page annotation inventory changed")
    link = identity(annots[2])
    return {"page": page, "name_field": name_field, "name_widget": name_widget, "keep_field": keep_field, "link": link}


def verify(source, candidate):
    before, after = source_checks(source), text_fields.inspect(candidate)
    old_reader, old = storage.inventory(source)
    reader, new = storage.inventory(candidate)
    copied = roles(new)
    require(set(old).issubset(new), "an original object was removed")
    expected_new = set(copied.values())
    require(len(expected_new) == 5 and all(number > 34 and generation == 0 for number, generation in expected_new), "copied roles are not five distinct new identities")
    require(set(new) - set(old) == expected_new, "unrequested new objects or copied immutable assets")
    require(storage.trailer(old_reader) == storage.trailer(reader), "meaningful trailer changed")
    mutable = {(2, 0), (7, 0), (31, 0)}
    for ref, value in old.items():
        if ref not in mutable:
            require(storage.typed(new[ref]) == storage.typed(value), f"unrequested original object change: {ref}")
        if isinstance(value, StreamObject):
            require(isinstance(new[ref], StreamObject) and new[ref]._data == value._data, f"original encoded stream changed: {ref}")
    same_except = text_fields.same_except
    same_except(old[(2, 0)], new[(2, 0)], {"/Kids", "/Count"})
    require([identity(value) for value in new[(2, 0)]["/Kids"]] == [(3, 0), copied["page"], (4, 0)] and new[(2, 0)]["/Count"] == 3, "source page-tree mapping/count differs")
    same_except(old[(7, 0)], new[(7, 0)], {"/Kids"})
    require([identity(value) for value in new[(7, 0)]["/Kids"]] == [(8, 0), (11, 0), copied["name_field"], copied["keep_field"]], "source sibling fields were removed or reordered")
    same_except(old[(31, 0)], new[(31, 0)], {"/Nums"})
    nums = old[(31, 0)]["/Nums"]
    expected_labels = ArrayObject([NumberObject(0), nums[1], NumberObject(1), nums[1], NumberObject(2), nums[3]])
    require(storage.typed(new[(31, 0)]["/Nums"]) == storage.typed(expected_labels), "labels did not preserve each source occurrence's explicit definition")
    for role in copied.values():
        require(isinstance(new[role], DictionaryObject) and not isinstance(new[role], StreamObject), "copied page/field/widget/link must remain a dictionary")
    page = new[copied["page"]]
    same_except(old[(3, 0)], page, {"/Annots"})
    require([identity(value) for value in page["/Annots"]] == [copied["name_widget"], copied["keep_field"], copied["link"]], "copied annotations not in original source order")
    name_field, name_widget, keep, link = (new[copied[key]] for key in ["name_field", "name_widget", "keep_field", "link"])
    same_except(old[(8, 0)], name_field, {"/T", "/Kids"})
    require(str(name_field["/T"]) == "name_copy" and [identity(value) for value in name_field["/Kids"]] == [copied["name_widget"]], "copied field name/widgets mismatch")
    same_except(old[(9, 0)], name_widget, {"/Parent", "/P"})
    require(identity(name_widget.raw_get("/Parent")) == copied["name_field"] and identity(name_widget.raw_get("/P")) == copied["page"], "copied name widget still belongs to an original field or page")
    same_except(old[(11, 0)], keep, {"/T", "/P"})
    require(str(keep["/T"]) == "keep_copy" and identity(keep.raw_get("/P")) == copied["page"], "copied combined field name/page mismatch")
    same_except(old[(18, 0)], link, {"/P"})
    require(identity(link.raw_get("/P")) == copied["page"], "copied link owns wrong page")
    require(identity(link["/Dest"][0]) == (4, 0), "copied destination was remapped away from original page4")
    require(after["pages"] == [3, copied["page"][0], 4] and after["labels"] == ["A-1", "A-1", "B-1"], "duplicate page/label inventory differs")
    expected_fields = {**before["fields"], **{new_name: before["fields"][old_name] for old_name, new_name in RENAMES.items()}}
    require(after["fields"] == expected_fields, "copied values/defaults differ or original registration changed")
    require(after["text"] == [before["text"][index] for index in MAPPING], "copied page content differs from its original")
    require(after["attachments"] == before["attachments"], "attachments changed")
    expected_destination = {name: 0 if index == 0 else index+1 for name, index in before["destinations"].items()}
    require(after["destinations"] == expected_destination and after["outline"] == [0 if index == 0 else index+1 for index in before["outline"]], "original navigation no longer targets original page identities")
    return {"source_objects_retained": len(old), "new_roles": {name: list(ref) for name, ref in copied.items()},
            "page_order": after["pages"], "source_page_mapping": MAPPING, "labels": after["labels"], "fields": after["fields"],
            "copied_annotations": [[9, copied["name_widget"][0]], [11, copied["keep_field"][0]], [18, copied["link"][0]]],
            "copied_link_target": 4, "original_encoded_streams": {str(ref): sha(value._data) for ref, value in old.items() if isinstance(value, StreamObject)}}


def identity_expected(source):
    def requested(objects):
        owner = objects[(3, 0)].indirect_reference.pdf
        ref = lambda number: IndirectObject(number, 0, owner)
        page, field, widget, keep, link = [DictionaryObject(objects[(number, 0)]) for number in [3, 8, 9, 11, 18]]
        page[NameObject("/Annots")] = ArrayObject([ref(37), ref(38), ref(39)])
        field[NameObject("/T")] = TextStringObject("name_copy")
        field[NameObject("/Kids")] = ArrayObject([ref(37)])
        widget[NameObject("/Parent")] = ref(36)
        widget[NameObject("/P")] = ref(35)
        keep[NameObject("/T")] = TextStringObject("keep_copy")
        keep[NameObject("/P")] = ref(35)
        link[NameObject("/P")] = ref(35)
        objects.update({(35, 0): page, (36, 0): field, (37, 0): widget, (38, 0): keep, (39, 0): link})
        objects[(2, 0)][NameObject("/Kids")] = ArrayObject([ref(3), ref(35), ref(4)])
        objects[(2, 0)][NameObject("/Count")] = NumberObject(3)
        objects[(7, 0)][NameObject("/Kids")] = ArrayObject([ref(8), ref(11), ref(36), ref(38)])
        nums = objects[(31, 0)]["/Nums"]
        objects[(31, 0)][NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[1], NumberObject(1), nums[1], NumberObject(2), nums[3]])
    return geometry.rewrite_objects(source, requested)


def independent_expected_pdf(source):
    writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(source), strict=True))
    originals = list(writer.pages)
    original_page = originals[0]
    page = writer.add_blank_page(width=300, height=260)
    page.clear()
    page.update(original_page)
    page_ref = page.indirect_reference
    group = writer.root_object["/AcroForm"]["/Fields"][0].get_object()
    name_source, keep_source = [value.get_object() for value in group["/Kids"]]
    copied_name = DictionaryObject(name_source)
    copied_name[NameObject("/T")] = TextStringObject("name_copy")
    field_ref = writer._add_object(copied_name)
    widget = DictionaryObject(name_source["/Kids"][0].get_object())
    widget[NameObject("/Parent")] = field_ref
    widget[NameObject("/P")] = page_ref
    widget_ref = writer._add_object(widget)
    copied_name[NameObject("/Kids")] = ArrayObject([widget_ref])
    keep = DictionaryObject(keep_source)
    keep[NameObject("/T")] = TextStringObject("keep_copy")
    keep[NameObject("/P")] = page_ref
    keep_ref = writer._add_object(keep)
    original_link = original_page["/Annots"][2].get_object()
    link = DictionaryObject(original_link)
    link[NameObject("/P")] = page_ref
    link_ref = writer._add_object(link)
    page[NameObject("/Annots")] = ArrayObject([widget_ref, keep_ref, link_ref])
    group[NameObject("/Kids")] = ArrayObject([*group["/Kids"], field_ref, keep_ref])
    root_pages = writer.root_object.raw_get("/Pages")
    root_pages.get_object()[NameObject("/Kids")] = ArrayObject([originals[0].indirect_reference, page_ref, originals[1].indirect_reference])
    root_pages.get_object()[NameObject("/Count")] = NumberObject(3)
    page[NameObject("/Parent")] = root_pages
    labels = writer.root_object["/PageLabels"]
    nums = labels["/Nums"]
    labels[NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[1], NumberObject(1), nums[1], NumberObject(2), nums[3]])
    writer.flattened_pages = [originals[0], page, originals[1]]
    output = io.BytesIO()
    writer.write(output)
    expected = output.getvalue()
    actual = text_fields.inspect(expected)
    require(actual["labels"] == ["A-1", "A-1", "B-1"] and set(actual["fields"]) == set(text_fields.inspect(source)["fields"]) | set(RENAMES.values()), "independent author page/field result differs")
    return expected


def negative_controls(source, candidate):
    _, original = storage.inventory(candidate)
    baseline = {ref: storage.typed(value) for ref, value in original.items()}
    def lost_sibling(objects):
        kids = objects[(7, 0)]["/Kids"]
        objects[(7, 0)][NameObject("/Kids")] = ArrayObject([kids[0], kids[2], kids[3]])
    def wrong_parent(objects):
        copied = roles(objects)
        objects[copied["name_widget"]][NameObject("/Parent")] = objects[(8, 0)].indirect_reference
    def original_widget_redirected(objects):
        objects[(9, 0)][NameObject("/P")] = objects[roles(objects)["page"]].indirect_reference
    def wrong_destination(objects):
        copied = roles(objects)
        objects[copied["link"]][NameObject("/Dest")] = ArrayObject([objects[copied["page"]].indirect_reference, NameObject("/Fit")])
    def wrong_label(objects):
        values = objects[(31, 0)]["/Nums"]
        copied = DictionaryObject(values[3])
        copied[NameObject("/St")] = NumberObject(2)
        values[3] = copied
    def changed_appearance(objects):
        appearance = objects[(14, 0)]
        appearance._data = zlib.compress(appearance.get_data().replace(b"(Old) Tj", b"(Bad) Tj"))
    def missing_link(objects):
        copied = roles(objects)
        objects[copied["page"]][NameObject("/Annots")] = ArrayObject(objects[copied["page"]]["/Annots"][:2])
    def copied_value(objects):
        objects[roles(objects)["name_field"]][NameObject("/V")] = TextStringObject("Bad")
    def copied_contents(objects):
        objects[roles(objects)["page"]][NameObject("/Contents")] = objects[(6, 0)].indirect_reference
    controls, negatives = {}, {}
    for name, mutate in [("lost_original_sibling", lost_sibling), ("copied_widget_original_parent", wrong_parent),
                         ("original_widget_redirected", original_widget_redirected), ("copied_link_retargeted", wrong_destination),
                         ("wrong_duplicate_label", wrong_label), ("changed_shared_appearance", changed_appearance),
                         ("missing_copied_link", missing_link), ("copied_value_changed", copied_value), ("copied_content_changed", copied_contents)]:
        corrupted = geometry.rewrite_objects(candidate, mutate)
        _, corrupted_objects = storage.inventory(corrupted)
        changed = [list(ref) for ref, value in corrupted_objects.items() if storage.typed(value) != baseline.get(ref)]
        require(changed, f"control {name} did not change a typed object")
        require(len(pypdf.PdfReader(io.BytesIO(corrupted), strict=True).pages) == 3, f"control {name} cannot reparse")
        try:
            verify(source, corrupted)
        except RuntimeError as error:
            controls[name] = {"reparse": "passed", "detected": str(error), "changed_objects": changed, "sha256": sha(corrupted)}
        else:
            raise RuntimeError(f"oracle accepted {name}")
        negatives[name] = corrupted
    return controls, negatives


def render_checks(tool, source, paths, negatives, report):
    from PIL import Image, ImageChops, __version__ as pillow_version
    tool = tool.resolve(strict=True)
    work = REPO / "target/binas-validation/duplicate"
    work.mkdir(parents=True, exist_ok=True)
    expected = independent_expected_pdf(source)
    expected_path = work / "independent-expected.pdf"
    expected_path.write_bytes(expected)
    inputs = {"expected": expected_path, "source": SOURCE, **paths}
    for name in ["changed_shared_appearance", "missing_copied_link", "copied_content_changed"]:
        path = work/f"negative-{name}.pdf"
        path.write_bytes(negatives[name])
        inputs[name] = path
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    images, records = {}, []
    for name, path in inputs.items():
        images[name] = []
        for page in range(1, 3 if name == "source" else 4):
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
    comparisons["source_occurrence_mapping"] = [same(images["expected"][index], images["source"][source_index]) for index, source_index in enumerate(MAPPING)]
    require(comparisons["source_occurrence_mapping"] == [True]*3, "independent duplicate author changed source page appearance")
    for name in paths:
        require(comparisons[name] == [True]*3, f"{name} differs from independent all-page RGB144 expected")
    for name, expected_match in [("changed_shared_appearance", [False]*3), ("missing_copied_link", [True, False, True]), ("copied_content_changed", [True, False, True])]:
        require(comparisons[name] == expected_match, f"{name} did not change exactly the expected rendered pages")
    report["rendering"] = {"result": "passed", "dpi": 144, "color": "RGB", "all_pages": True, "comparisons": comparisons,
                           "expected_pdf_sha256": sha(expected), "expected_authorship": "independent pypdf source clone, explicit copied page/field/widget/link ownership and occurrence labels",
                           "pillow": pillow_version, "poppler": (version.stdout+version.stderr).strip(), "renderer_sha256": sha(tool.read_bytes()), "renders": records}


def record_source():
    source = SOURCE.read_bytes()
    source_record = json.loads(SOURCE_PROVENANCE.read_text(encoding="utf-8"))
    require(sha(source) == source_record["sha256"] and source == text_fields.fixture_bytes(), "existing frozen source differs from its provenance/generator")
    source_checks(source)
    require(not oracle.WARNINGS and not storage.WARNINGS, "source reader warnings")
    PROVENANCE.write_text(json.dumps({"schema": 1, "license": "Apache-2.0", "origin": "Reuses the original Binas text-field-fill.pdf fixture unchanged; no derivative PDF generated.",
        "source_path": "fixtures/text-field-fill.pdf", "source_sha256": sha(source), "source_bytes": len(source),
        "source_provenance_path": "fixtures/text-field-fill.provenance.json", "source_provenance_sha256": sha(SOURCE_PROVENANCE.read_bytes()),
        "source_generator_sha256": source_record["generator_sha256"], "oracle_sha256": sha(Path(__file__).read_bytes()),
        "python": platform.python_version(), "pypdf": pypdf.__version__, "request": {"page_index": 0, "field_names": RENAMES},
        "source_occurrence_mapping": MAPPING, "original_destination_retained": 4, "expected_new_objects": 5,
        "assurance": "Source identity/reader check only; independent self-check and actual engine outputs are separate gates."}, indent=2)+"\n", encoding="utf-8")
    print(json.dumps({"source_sha256": sha(source), "source_unchanged": True}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--record-source", action="store_true")
    mode.add_argument("--structure-only", action="store_true")
    mode.add_argument("--pdftoppm", type=Path)
    args = parser.parse_args()
    require(pypdf.__version__ == "6.10.0", "install pinned pypdf6.10.0")
    if args.record_source:
        record_source()
        return
    source = SOURCE.read_bytes()
    provenance = json.loads(PROVENANCE.read_text(encoding="utf-8"))
    require(sha(source) == provenance["source_sha256"] and source == text_fields.fixture_bytes(), "frozen duplication source mismatch")
    require(sha(SOURCE_PROVENANCE.read_bytes()) == provenance["source_provenance_sha256"], "source provenance changed")
    require(sha(Path(__file__).read_bytes()) == provenance["oracle_sha256"], "duplication oracle changed; review provenance")
    require(not oracle.WARNINGS and not storage.WARNINGS, "warnings before scoped gate")
    phases, outputs = [], []
    previous = flatten.checkpoint()
    source_checks(source)
    phases.append(flatten.warning_phase("source", previous))
    paths = {name: ROOT/f"evidence/duplicate-{name}.pdf" for name in ["incremental", "compact"]}
    for name, path in paths.items():
        previous = flatten.checkpoint()
        candidate = path.read_bytes()
        if name == "incremental":
            require(candidate.startswith(source), "incremental original prefix changed")
        outputs.append({"mode": name, "bytes": len(candidate), "sha256": sha(candidate), "checks": verify(source, candidate)})
        phases.append(flatten.warning_phase(name, previous, source, candidate))
    previous = flatten.checkpoint()
    controls, negatives = negative_controls(source, paths["compact"].read_bytes())
    phases.append(flatten.warning_phase("corruption_controls", previous))
    warning_controls = flatten.warning_gate_controls(source, paths["incremental"].read_bytes())
    report = {"schema": 1, "result": "passed", "scope": __doc__.strip(), "python": platform.python_version(), "pypdf": pypdf.__version__,
              "source_sha256": sha(source), "oracle_sha256": sha(Path(__file__).read_bytes()), "provenance_sha256": sha(PROVENANCE.read_bytes()),
              "example_sha256": sha((REPO/"crates/binas-pdf/examples/preservation_duplicate.rs").read_bytes()),
              "helper_sha256": {name: sha((ROOT/name).read_bytes()) for name in ["text_field_oracle.py", "flatten_oracle.py", "flate_oracle.py", "geometry_oracle.py", "oracle.py", "requirements.txt", "LICENSE"]},
              "request": {"page_index": 0, "field_names": RENAMES}, "outputs": outputs, "negative_controls": controls, "warning_gate_controls": warning_controls,
              "ignored_layout": ["xref offsets and representation", "trailer Size/Prev/XRefStm", "new role object numbering"], "rendering": {"result": "not_checked"}}
    if args.pdftoppm:
        previous = flatten.checkpoint()
        render_checks(args.pdftoppm, source, paths, negatives, report)
        phases.append(flatten.warning_phase("independent_render_expectation", previous))
    require([warning for phase in phases for warning in phase["warnings"]] == oracle.WARNINGS == storage.WARNINGS, "reader warning escaped phase accounting")
    report["reader_warnings"] = phases
    name = "duplicate-evidence.json" if args.pdftoppm else "duplicate-structure.json"
    (ROOT/"evidence"/name).write_text(json.dumps(report, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "writer_outputs": 2, "corruption_controls": len(controls), "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
