"""Local all-page import appearance proof for the two frozen Type3 fixtures.

Exact RGB144/CropBox comparison through every requested import mapping. This is
synthetic fixture appearance evidence, not broad PDF or color fidelity assurance.
"""
import argparse
import json
import platform
import subprocess
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, __version__ as pillow_version

import import_oracle as oracle


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pdftoppm", type=Path, required=True)
    args = parser.parse_args()
    tool = args.pdftoppm.resolve(strict=True)
    checker = oracle.checker()
    oracle.sources()  # Pins both mandatory source hashes and source semantics.
    raw = oracle.REPO / "target/binas-validation/import-render"
    raw.mkdir(parents=True, exist_ok=True)
    version = subprocess.run([str(tool), "-v"], check=True, capture_output=True, text=True, timeout=30)
    report = {
        "schema": 1,
        "scope": "Local exact RGB rendering of all 15 requested output pages from two original generated Type3 fixtures; no broad PDF, standards or print-color guarantee",
        "renderer": (version.stdout + version.stderr).strip(),
        "renderer_executable_sha256": oracle.sha(tool.read_bytes()),
        "renderer_version_command": [str(tool), "-v"],
        "checker_sha256": oracle.sha(Path(__file__).read_bytes()),
        "mapping_checker": checker,
        "pillow": pillow_version,
        "python": platform.python_version(),
        "platform": platform.platform(),
        "dpi": 144, "color": "RGB", "cropbox": True,
        "rotation": "Intrinsic PDF rotation honored by Poppler",
        "source_sha256": oracle.SOURCE_HASHES,
        "renders": [], "comparisons": [],
    }

    def render(path, name, page_count):
        path = path.resolve(strict=True)
        rendered = []
        page_evidence = []
        for number in range(1, page_count + 1):
            prefix = raw / f"{name}-{number}"
            command = [str(tool), "-f", str(number), "-l", str(number), "-singlefile", "-r", "144", "-cropbox", "-png", str(path), str(prefix)]
            result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
            if result.stderr.strip():
                raise RuntimeError(f"Unexpected renderer warning: {result.stderr}")
            png = prefix.with_suffix(".png")
            with Image.open(png) as image:
                rgb = image.convert("RGB")
            rendered.append(rgb)
            page_evidence.append({
                "page": number, "size": list(rgb.size),
                "rgb_sha256": oracle.sha(rgb.tobytes()),
                "png_sha256": oracle.sha(png.read_bytes()),
                "png": str(png.relative_to(oracle.REPO)),
                "command": command, "stderr": result.stderr,
            })
        report["renders"].append({"name": name, "pdf": str(path.relative_to(oracle.REPO)), "pdf_sha256": oracle.sha(path.read_bytes()), "pages": page_evidence})
        return rendered

    def compare(source, candidate):
        same_size = source.size == candidate.size
        changed = None
        if same_size:
            red, green, blue = ImageChops.difference(source, candidate).split()
            maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
            changed = source.width * source.height - maximum.histogram()[0]
        return {"same_size": same_size, "changed_rgb_pixels": changed, "passed": same_size and changed == 0}

    sources = {name: render(oracle.FIXTURES / name, name.removesuffix(".pdf"), 2) for name in oracle.SOURCE_HASHES}
    sheet_rows = []
    for name, mapping in oracle.CASES.items():
        output = render(oracle.EVIDENCE / f"{name}.pdf", name, len(mapping))
        source = [sources[fixture][index] for fixture, index in mapping]
        comparisons = [compare(before, after) | {"source_fixture": fixture, "source_page_index": old, "output_page_index": new} for new, ((fixture, old), before, after) in enumerate(zip(mapping, source, output))]
        if not all(item["passed"] for item in comparisons):
            raise RuntimeError(f"Import appearance mismatch: {name}: {comparisons}")
        report["comparisons"].append({"case": name, "pages": comparisons})
        sheet_rows.append((name + " - requested source mapping", source))
        sheet_rows.append((name + " - Binas output, exact RGB match", output))

    original = oracle.EVIDENCE / "import-shared-duplicate.pdf"
    data = original.read_bytes()
    old = b"600 0 0 0 600 700 d1 0 0 600 700 re f"
    new = b"600 0 0 0 600 700 d1 0 0 300 700 re f"
    if data.count(old) != 1 or len(old) != len(new):
        raise RuntimeError("Shared-glyph corruption control encoding changed")
    corrupt = raw / "shared-glyph-half-width.pdf"
    corrupt.write_bytes(data.replace(old, new))
    if len(oracle.reader(corrupt).pages) != 3:
        raise RuntimeError("Glyph control must remain independently reparseable")
    images = render(corrupt, "control-shared-glyph-half-width", 3)
    mapping = oracle.CASES["import-shared-duplicate"]
    control = [compare(sources[fixture][old], image) | {"source_fixture": fixture, "source_page_index": old, "output_page_index": new} for new, ((fixture, old), image) in enumerate(zip(mapping, images))]
    if not all(item["same_size"] and item["changed_rgb_pixels"] > 0 for item in control):
        raise RuntimeError("Shared-glyph mutation must visibly change all three affected pages")
    report["corruption_control"] = {"mutation": "Same-length glyph rectangle width 600 to 300; xrefs/stream lengths unchanged", "independent_reparse": "passed", "all_affected_pages_detected": True, "pages": control}
    sheet_rows.append(("CONTROL - shared glyph half-width; all three pages differ", images))
    sheet = Image.new("RGB", (1200, len(sheet_rows) * 250), "#e8e8e8")
    draw = ImageDraw.Draw(sheet)
    for row, (label, images) in enumerate(sheet_rows):
        draw.text((8, row * 250 + 6), label, fill="black")
        for column, image in enumerate(images):
            thumb = image.copy()
            thumb.thumbnail((220, 218))
            sheet.paste(thumb, (8 + column * 240, row * 250 + 28))
    contact = oracle.EVIDENCE / "import-render-contactsheet.png"
    sheet.save(contact)
    report |= {"result": "15_page_comparisons_pixel_exact_and_3_control_pages_detected", "contact_sheet": str(contact.relative_to(oracle.REPO)), "contact_sheet_sha256": oracle.sha(contact.read_bytes())}
    if oracle.WARNINGS:
        raise RuntimeError(f"Unexpected independent parser warning: {oracle.WARNINGS}")
    destination = oracle.EVIDENCE / "import-render.json"
    destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(report["result"])
    print(contact)


if __name__ == "__main__":
    main()
