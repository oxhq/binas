"""Original-source independent gate for retained logical page deletion.

Delete original page4 (index1), explicitly remapping its local Fit destinations
to retained page3. Remove only the excluded widgets' active registration; retain
the surviving field values/defaults, appearances, labels and document assets.
All original objects remain stored. This is not secure erasure, arbitrary page
extraction, tag reconciliation or general action/form preservation evidence.
"""
import argparse
import io
import json
import platform
import subprocess
import zlib
from pathlib import Path

import pypdf
from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, NameObject, NumberObject, StreamObject, TextStringObject

import flate_oracle as storage
import flatten_oracle as flatten
import geometry_oracle as geometry
import oracle
import text_field_oracle as text_fields

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
SOURCE = ROOT / "fixtures/text-field-fill.pdf"
SOURCE_PROVENANCE = ROOT / "fixtures/text-field-fill.provenance.json"
PROVENANCE = ROOT / "fixtures/page-delete.provenance.json"
MAPPING = [0]
REQUEST = {"page_index": 1, "excluded_targets": {"remap_to_page_index": 0}}
CHANGED = {(number, 0) for number in [2, 8, 18, 23, 26, 27, 31]}
DETACHED = {(number, 0) for number in [4, 6, 10, 12, 13, 15, 16, 17]}
sha, require = storage.sha, storage.require
identity = flatten.raw_identity


def source_checks(source):
    result = text_fields.source_checks(source)
    _, objects = storage.inventory(source)
    require(len(objects) == 34, "source object inventory changed")
    require([identity(value) for value in objects[(3, 0)]["/Annots"]] == [(9, 0), (11, 0), (18, 0)], "source retained annotations changed")
    require([identity(value) for value in objects[(4, 0)]["/Annots"]] == [(10, 0), (12, 0), (13, 0)], "source excluded annotations changed")
    for target in [objects[(18, 0)]["/Dest"], objects[(26, 0)]["/Dest"], objects[(27, 0)]["/Dests"]["/Names"][1]]:
        require(storage.typed(target) == ["array", [["ref", 4, 0], ["name", "/Fit"]]], "source excluded local Fit destination changed")
    return result


def active_objects(reader, objects):
    """Follow raw trailer/dictionary/array references, never object-store order."""
    seen = set()

    def visit(value):
        if isinstance(value, IndirectObject):
            ref = identity(value)
            require(ref in objects, f"active graph contains missing object {ref}")
            if ref not in seen:
                seen.add(ref)
                visit(objects[ref])
        elif isinstance(value, DictionaryObject):
            for item in value.values():
                visit(item)
        elif isinstance(value, ArrayObject):
            for item in value:
                visit(item)

    visit(reader.trailer)
    return seen


def verify(source, candidate):
    before = source_checks(source)
    source_reader, old = storage.inventory(source)
    reader, new = storage.inventory(candidate)
    require(set(new) == set(old), "logical deletion must retain all original stored objects and add none")
    require(storage.trailer(source_reader) == storage.trailer(reader), "meaningful trailer changed")
    for ref, value in old.items():
        if ref not in CHANGED:
            require(storage.typed(value) == storage.typed(new[ref]), f"unrequested original object change: {ref}")
        if isinstance(value, StreamObject):
            require(isinstance(new[ref], StreamObject) and value._data == new[ref]._data, f"original encoded stream changed: {ref}")
    same_except = text_fields.same_except
    same_except(old[(2, 0)], new[(2, 0)], {"/Kids", "/Count"})
    require([identity(value) for value in new[(2, 0)]["/Kids"]] == [(3, 0)] and new[(2, 0)]["/Count"] == 1, "retained page tree mapping/count differs")
    same_except(old[(8, 0)], new[(8, 0)], {"/Kids"})
    require([identity(value) for value in new[(8, 0)]["/Kids"]] == [(9, 0)], "excluded widget remains registered or retained widget was removed")
    same_except(old[(23, 0)], new[(23, 0)], {"/Fields"})
    require([identity(value) for value in new[(23, 0)]["/Fields"]] == [(7, 0)], "removed-page terminal fields remain registered or surviving group was lost")
    destination = ["array", [["ref", 3, 0], ["name", "/Fit"]]]
    for number in [18, 26]:
        same_except(old[(number, 0)], new[(number, 0)], {"/Dest"})
        require(storage.typed(new[(number, 0)]["/Dest"]) == destination, f"explicit local Fit remap missing on object {number}")
    same_except(old[(27, 0)], new[(27, 0)], {"/Dests"})
    old_dests, new_dests = old[(27, 0)]["/Dests"], new[(27, 0)]["/Dests"]
    same_except(old_dests, new_dests, {"/Names"})
    names = new_dests["/Names"]
    require(len(names) == 2 and storage.typed(names[0]) == storage.typed(old_dests["/Names"][0])
            and storage.typed(names[1]) == destination, "named destination key or explicit local Fit remap differs")
    same_except(old[(31, 0)], new[(31, 0)], {"/Nums"})
    expected_labels = ArrayObject([NumberObject(0), old[(31, 0)]["/Nums"][1]])
    require(storage.typed(new[(31, 0)]["/Nums"]) == storage.typed(expected_labels), "retained source label definition/private data differs")
    reachable = active_objects(reader, new)
    require(active_objects(source_reader, old) == set(old), "frozen source has unexpectedly detached objects")
    require(set(old) - reachable == DETACHED, "excluded page/widget closure remains active or unrelated content became detached")
    after = text_fields.inspect(candidate)
    require(after["pages"] == [3] and after["labels"] == ["A-1"], "retained page/label inventory differs")
    require(after["fields"] == {name: before["fields"][name] for name in ["group", "group.name", "group.keep"]}, "surviving field registration/values/defaults differ")
    require(after["text"] == [before["text"][0]], "retained page text changed")
    require(after["outline"] == [0] and after["destinations"] == {"bravo": 0}, "remapped outline/named destinations do not target the retained page")
    require(after["attachments"] == before["attachments"], "attachments changed")
    return {"source_objects_retained": len(old), "changed_objects": [list(ref) for ref in sorted(CHANGED)],
            "active_objects": [list(ref) for ref in sorted(reachable)], "detached_stored_objects": [list(ref) for ref in sorted(DETACHED)],
            "page_order": after["pages"], "source_page_mapping": MAPPING, "labels": after["labels"], "fields": after["fields"],
            "unregistered_widgets": [10, 12, 13], "unregistered_terminal_fields": [12, 13], "surviving_name_widgets": [9],
            "destination_remaps": [{"owner": 18, "key": "Dest", "before_page": 4, "after_page": 3},
                                   {"owner": 26, "key": "Dest", "before_page": 4, "after_page": 3},
                                   {"owner": 27, "key": "Dests/Names/bravo", "before_page": 4, "after_page": 3}],
            "original_encoded_streams": {str(ref): sha(value._data) for ref, value in old.items() if isinstance(value, StreamObject)},
            "secure_erasure": "not_performed_original_detached_objects_retained"}


def identity_expected(source):
    """Independent pypdf value writer with the source identities retained."""
    def requested(objects):
        page = objects[(3, 0)].indirect_reference
        objects[(2, 0)][NameObject("/Kids")] = ArrayObject([page])
        objects[(2, 0)][NameObject("/Count")] = NumberObject(1)
        objects[(8, 0)][NameObject("/Kids")] = ArrayObject([objects[(9, 0)].indirect_reference])
        objects[(23, 0)][NameObject("/Fields")] = ArrayObject([objects[(7, 0)].indirect_reference])
        for number in [18, 26]:
            objects[(number, 0)][NameObject("/Dest")] = ArrayObject([page, NameObject("/Fit")])
        dests = objects[(27, 0)]["/Dests"]
        dests[NameObject("/Names")] = ArrayObject([dests["/Names"][0], ArrayObject([page, NameObject("/Fit")])])
        nums = objects[(31, 0)]["/Nums"]
        objects[(31, 0)][NameObject("/Nums")] = ArrayObject([NumberObject(0), nums[1]])
    return geometry.rewrite_objects(source, requested)


def independent_expected_pdf(source):
    """Separate visual author using cloned semantic roles rather than fixed IDs."""
    writer = pypdf.PdfWriter(clone_from=pypdf.PdfReader(io.BytesIO(source), strict=True))
    original_pages = list(writer.pages)
    retained, excluded = original_pages
    page = retained.indirect_reference
    catalog = writer.root_object
    tree = catalog["/Pages"]
    tree[NameObject("/Kids")] = ArrayObject([page])
    tree[NameObject("/Count")] = NumberObject(1)
    form = catalog["/AcroForm"]
    group_ref = form["/Fields"][0]
    form[NameObject("/Fields")] = ArrayObject([group_ref])
    name_field = group_ref.get_object()["/Kids"][0].get_object()
    kept_widgets = [widget for widget in name_field["/Kids"] if identity(widget.get_object().raw_get("/P")) != identity(excluded.indirect_reference)]
    require(len(kept_widgets) == 1, "independent author did not select one surviving name widget")
    name_field[NameObject("/Kids")] = ArrayObject(kept_widgets)
    link = retained["/Annots"][2].get_object()
    link[NameObject("/Dest")] = ArrayObject([page, NameObject("/Fit")])
    outline = catalog["/Outlines"]["/First"]
    outline[NameObject("/Dest")] = ArrayObject([page, NameObject("/Fit")])
    dests = catalog["/Names"]["/Dests"]
    dests[NameObject("/Names")] = ArrayObject([dests["/Names"][0], ArrayObject([page, NameObject("/Fit")])])
    labels = catalog["/PageLabels"]
    labels[NameObject("/Nums")] = ArrayObject([NumberObject(0), labels["/Nums"][1]])
    writer.flattened_pages = [retained]
    output = io.BytesIO()
    writer.write(output)
    expected = output.getvalue()
    before, after = text_fields.inspect(source), text_fields.inspect(expected)
    require(len(after["pages"]) == 1 and after["labels"] == ["A-1"] and after["text"] == [before["text"][0]], "independent author page/text/label result differs")
    require(after["fields"] == {name: before["fields"][name] for name in ["group", "group.name", "group.keep"]}, "independent author surviving fields differ")
    require(after["outline"] == [0] and after["destinations"] == {"bravo": 0} and after["attachments"] == before["attachments"], "independent author navigation/attachment result differs")
    return expected


def negative_controls(source, candidate):
    _, original = storage.inventory(candidate)
    baseline = {ref: storage.typed(value) for ref, value in original.items()}
    def leftover_widget(objects):
        objects[(8, 0)][NameObject("/Kids")] = ArrayObject([objects[(number, 0)].indirect_reference for number in [9, 10]])
    def lost_field(objects):
        objects[(7, 0)][NameObject("/Kids")] = ArrayObject([objects[(8, 0)].indirect_reference])
    def stale_destination(objects, owner):
        objects[(owner, 0)][NameObject("/Dest")] = ArrayObject([objects[(4, 0)].indirect_reference, NameObject("/Fit")])
    def stale_named_destination(objects):
        names = objects[(27, 0)]["/Dests"]["/Names"]
        names[1] = ArrayObject([objects[(4, 0)].indirect_reference, NameObject("/Fit")])
    def attachment(objects):
        objects[(29, 0)]._data = b"Unrequested replacement attachment"
    def missing_label(objects):
        del objects[(1, 0)]["/PageLabels"]
    def label_private(objects):
        del objects[(31, 0)]["/Nums"][1]["/Private"]
    def changed_value(objects):
        objects[(8, 0)][NameObject("/V")] = TextStringObject("Bad")
    def appearance(objects):
        value = objects[(14, 0)]
        value._data = zlib.compress(value.get_data().replace(b"(Old) Tj", b"(Bad) Tj"))
    def missing_link(objects):
        objects[(3, 0)][NameObject("/Annots")] = ArrayObject(objects[(3, 0)]["/Annots"][:2])
    def leftover_root(objects):
        objects[(23, 0)][NameObject("/Fields")] = ArrayObject([objects[(number, 0)].indirect_reference for number in [7, 12]])
    controls, negatives = {}, {}
    for name, mutate in [("leftover_excluded_widget", leftover_widget), ("lost_surviving_field", lost_field),
                         ("stale_link_target", lambda objects: stale_destination(objects, 18)),
                         ("stale_outline_target", lambda objects: stale_destination(objects, 26)),
                         ("stale_named_target", stale_named_destination), ("attachment_changed", attachment),
                         ("page_labels_removed", missing_label), ("label_private_data_lost", label_private),
                         ("surviving_value_changed", changed_value), ("shared_appearance_changed", appearance),
                         ("retained_link_removed", missing_link), ("excluded_root_still_registered", leftover_root)]:
        corrupted = geometry.rewrite_objects(candidate, mutate)
        _, objects = storage.inventory(corrupted)
        changed = [list(ref) for ref, value in objects.items() if storage.typed(value) != baseline[ref]]
        require(len(changed) == 1, f"control {name} did not change exactly one object")
        require(len(pypdf.PdfReader(io.BytesIO(corrupted), strict=True).pages) == 1, f"control {name} cannot reparse")
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
    work = REPO / "target/binas-validation/delete"
    work.mkdir(parents=True, exist_ok=True)
    expected = independent_expected_pdf(source)
    expected_path = work / "independent-expected.pdf"
    expected_path.write_bytes(expected)
    inputs = {"expected": expected_path, "source": SOURCE, **paths}
    for name in ["shared_appearance_changed", "retained_link_removed"]:
        path = work/f"negative-{name}.pdf"
        path.write_bytes(negatives[name])
        inputs[name] = path
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    images, records = {}, []
    for name, path in inputs.items():
        images[name] = []
        for page in range(1, 3 if name == "source" else 2):
            prefix = work/f"{name}-{page}"
            command = [str(tool), "-f", str(page), "-l", str(page), "-singlefile", "-r", "144", "-cropbox", "-png", str(path), str(prefix)]
            result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
            known = {"Syntax Error: No display font for 'Symbol'", "Syntax Error: No display font for 'ArialUnicode'"}
            require(set(result.stderr.splitlines()) <= known, f"unacknowledged renderer diagnostic for {name}: {result.stderr}")
            png = Path(str(prefix)+".png")
            with Image.open(png) as opened:
                image = opened.convert("RGB")
            images[name].append(image)
            records.append({"name": name, "page": page, "command": command, "stderr": result.stderr, "size": list(image.size),
                            "rgb_sha256": sha(image.tobytes()), "png_sha256": sha(png.read_bytes())})
    def same(left, right):
        return left.size == right.size and ImageChops.difference(left, right).getbbox() is None
    comparisons = {name: [same(images["expected"][0], images[name][0])] for name in inputs if name not in {"expected", "source"}}
    comparisons["source_page_mapping"] = [same(images["expected"][0], images["source"][0])]
    require(comparisons["source_page_mapping"] == [True], "independent author changed the retained source page appearance")
    for name in paths:
        require(comparisons[name] == [True], f"{name} differs from independent all-output-page RGB144 expected")
    for name in ["shared_appearance_changed", "retained_link_removed"]:
        require(comparisons[name] == [False], f"{name} did not change retained page appearance")
    diagnostics = {record["stderr"] for record in records}
    require(len(diagnostics) == 1, "renderer diagnostics differ from independent expectation/source")
    report["rendering"] = {"result": "passed", "dpi": 144, "color": "RGB", "all_output_pages": True, "comparisons": comparisons,
                           "expected_pdf_sha256": sha(expected), "expected_authorship": "Independent pypdf source clone with explicit semantic-role page/widget removal, local Fit remaps and retained label",
                           "pillow": pillow_version, "poppler": (version.stdout+version.stderr).strip(), "renderer_sha256": sha(tool.read_bytes()),
                           "acknowledged_renderer_diagnostics": sorted(diagnostics), "renders": records}


def record_source():
    source = SOURCE.read_bytes()
    source_record = json.loads(SOURCE_PROVENANCE.read_text(encoding="utf-8"))
    require(sha(source) == source_record["sha256"] and source == text_fields.fixture_bytes(), "existing frozen source differs from its provenance/generator")
    source_checks(source)
    require(not oracle.WARNINGS and not storage.WARNINGS, "source reader warnings")
    PROVENANCE.write_text(json.dumps({"schema": 1, "license": "Apache-2.0", "origin": "Reuses the original Binas text-field-fill.pdf fixture unchanged; no derivative source PDF generated.",
        "source_path": "fixtures/text-field-fill.pdf", "source_sha256": sha(source), "source_bytes": len(source),
        "source_provenance_path": "fixtures/text-field-fill.provenance.json", "source_provenance_sha256": sha(SOURCE_PROVENANCE.read_bytes()),
        "source_generator_sha256": source_record["generator_sha256"], "oracle_sha256": sha(Path(__file__).read_bytes()),
        "python": platform.python_version(), "pypdf": pypdf.__version__, "request": REQUEST, "source_page_mapping": MAPPING,
        "removed_page_object": 4, "remap_target_page_object": 3, "surviving_name_widgets": [9], "unregistered_widgets": [10, 12, 13],
        "unregistered_terminal_fields": [12, 13], "expected_stored_objects": 34, "expected_new_objects": 0,
        "detached_stored_objects": [list(ref) for ref in sorted(DETACHED)], "secure_erasure": False,
        "assurance": "Source identity/reader check only. In-memory independent self-check and actual Rust export evidence are separate gates. Original detached objects and incremental history remain."}, indent=2)+"\n", encoding="utf-8")
    print(json.dumps({"source_sha256": sha(source), "source_unchanged": True, "rust_outputs_checked": False}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--record-source", action="store_true")
    mode.add_argument("--self-check", action="store_true")
    mode.add_argument("--structure-only", action="store_true")
    mode.add_argument("--pdftoppm", type=Path)
    args = parser.parse_args()
    require(pypdf.__version__ == "6.10.0", "install pinned pypdf6.10.0")
    if args.record_source:
        record_source()
        return
    source = SOURCE.read_bytes()
    provenance = json.loads(PROVENANCE.read_text(encoding="utf-8"))
    require(sha(source) == provenance["source_sha256"] and source == text_fields.fixture_bytes(), "frozen deletion source mismatch")
    require(sha(SOURCE_PROVENANCE.read_bytes()) == provenance["source_provenance_sha256"], "source provenance changed")
    require(sha(Path(__file__).read_bytes()) == provenance["oracle_sha256"], "deletion oracle changed; review provenance")
    require(not oracle.WARNINGS and not storage.WARNINGS, "warnings before scoped gate")
    if args.self_check:
        candidate = identity_expected(source)
        checks = verify(source, candidate)
        controls, _ = negative_controls(source, candidate)
        expected = independent_expected_pdf(source)
        require(not oracle.WARNINGS and not storage.WARNINGS, "independent self-check reader warnings")
        print(json.dumps({"result": "independent_self_check_passed", "rust_outputs_checked": False, "rendering": "not_checked",
                          "candidate_sha256": sha(candidate), "visual_expected_sha256": sha(expected), "corruption_controls": len(controls), "checks": checks}))
        return
    phases, outputs = [], []
    previous = flatten.checkpoint()
    source_checks(source)
    phases.append(flatten.warning_phase("source", previous))
    paths = {name: ROOT/f"evidence/delete-{name}.pdf" for name in ["incremental", "compact"]}
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
              "example_sha256": sha((REPO/"crates/binas-pdf/examples/preservation_delete.rs").read_bytes()),
              "helper_sha256": {name: sha((ROOT/name).read_bytes()) for name in ["text_field_oracle.py", "flatten_oracle.py", "flate_oracle.py", "geometry_oracle.py", "oracle.py", "requirements.txt", "LICENSE"]},
              "request": REQUEST, "outputs": outputs, "negative_controls": controls, "warning_gate_controls": warning_controls,
              "ignored_layout": ["xref offsets and representation", "trailer Size/Prev/XRefStm", "stream Length"], "rendering": {"result": "not_checked"}}
    if args.pdftoppm:
        previous = flatten.checkpoint()
        render_checks(args.pdftoppm, source, paths, negatives, report)
        phases.append(flatten.warning_phase("independent_render_expectation", previous))
    require([warning for phase in phases for warning in phase["warnings"]] == oracle.WARNINGS == storage.WARNINGS, "reader warning escaped phase accounting")
    report["reader_warnings"] = phases
    name = "delete-evidence.json" if args.pdftoppm else "delete-structure.json"
    (ROOT/"evidence"/name).write_text(json.dumps(report, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "writer_outputs": 2, "corruption_controls": len(controls), "rendering": report["rendering"]["result"]}))


if __name__ == "__main__":
    main()
