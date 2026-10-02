"""Local synthetic all-page RGB preservation gate; requires Poppler and Pillow.
No standard conformance, semantic accessibility, or broad compatibility claim.
Original script, Apache-2.0. Missing tools/inputs and mismatches are fatal.
"""
import argparse
import hashlib
import json
import platform
import subprocess
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, __version__ as pillow_version

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
ORDER = [2, 0, 1]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pdftoppm", required=True, type=Path)
    parser.add_argument("--dpi", type=int, default=144, choices=[144, 200])
    args = parser.parse_args()
    tool = args.pdftoppm.resolve(strict=True)
    version = subprocess.run([str(tool), "-v"], capture_output=True, text=True, check=True)
    raw = REPO / "target/binas-validation/render"
    raw.mkdir(parents=True, exist_ok=True)
    report = {"schema": 1, "scope": "Local synthetic RGB rendering only; all three pages of two fixtures, both writers. No standards, semantic, or whole-corpus assurance.",
              "renderer": (version.stdout + version.stderr).strip(), "pillow": pillow_version,
              "python": platform.python_version(), "platform": platform.platform(), "dpi": args.dpi,
              "color": "RGB", "cropbox": True, "rotation": "PDF intrinsic rotation honored by Poppler",
              "requested_source_page_indices": ORDER, "renders": [], "comparisons": []}

    def render(pdf, name):
        pdf = pdf.resolve(strict=True)
        prefix = raw / name
        # One invocation per known page avoids stale glob matches and verifies all three outputs.
        images = []
        pages = []
        for page in range(1, 4):
            out = Path(str(prefix) + f"-{page}")
            command = [str(tool), "-f", str(page), "-l", str(page), "-singlefile", "-r", str(args.dpi),
                       "-cropbox", "-png", str(pdf), str(out)]
            result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=60)
            png = Path(str(out) + ".png")
            with Image.open(png) as loaded:
                image = loaded.convert("RGB")
            images.append(image)
            pages.append({"page": page, "size": list(image.size), "rgb_sha256": sha(image.tobytes()),
                          "png_sha256": sha(png.read_bytes()), "png": str(png.relative_to(REPO)),
                          "command": command, "stderr": result.stderr})
        report["renders"].append({"name": name, "pdf": str(pdf.relative_to(REPO)),
                                  "pdf_sha256": sha(pdf.read_bytes()), "pages": pages})
        return images

    def compare(source, output):
        results = []
        for new, old in enumerate(ORDER):
            a, b = source[old], output[new]
            same_size = a.size == b.size
            changed = None
            if same_size:
                red, green, blue = ImageChops.difference(a, b).split()
                maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
                changed = a.width * a.height - maximum.histogram()[0]
            results.append({"source_page": old + 1, "output_page": new + 1,
                            "same_size": same_size, "changed_rgb_pixels": changed,
                            "passed": same_size and changed == 0})
        return results

    manifest = json.loads((ROOT / "manifest.json").read_text())
    for entry in manifest["required"]:
        if sha((ROOT / entry["path"]).read_bytes()) != entry["sha256"]:
            raise RuntimeError("Mandatory source fixture hash mismatch")
    sheet_rows = []
    for family, source_name, candidate_prefix in [("main", "reorder-structures", "reordered"), ("alpha", "alpha-boundary", "alpha")]:
        source = render(ROOT / f"fixtures/{source_name}.pdf", family + "-source")
        sheet_rows.append((family + " source mapped [3,1,2]", [source[i] for i in ORDER]))
        for mode in ["compact", "incremental"]:
            candidate = render(ROOT / f"evidence/{candidate_prefix}-{mode}.pdf", family + "-" + mode)
            comparisons = compare(source, candidate)
            report["comparisons"].append({"family": family, "mode": mode, "pages": comparisons})
            if not all(item["passed"] for item in comparisons):
                raise RuntimeError(f"Visual preservation failed: {family}/{mode}: {comparisons}")
            sheet_rows.append((family + " " + mode, candidate))
        if family == "main":
            data = (ROOT / "evidence/reordered-compact.pdf").read_bytes()
            needle = b"stream\n\xff\x00\x00\nendstream"
            if data.count(needle) != 1:
                raise RuntimeError("Visual corruption control image encoding changed")
            corrupted = raw / "red-image-changed-to-green.pdf"
            corrupted.write_bytes(data.replace(needle, b"stream\n\x00\xff\x00\nendstream"))
            corrupted_images = render(corrupted, "corrupt-image")
            control = compare(source, corrupted_images)
            if all(item["passed"] for item in control):
                raise RuntimeError("Visual negative control was not detected")
            report["negative_control"] = {"effect": "Same-length red-to-green shared image sample change; offsets and stream lengths untouched", "pages": control, "detected": True}
            sheet_rows.append(("CONTROL: shared image changed green", corrupted_images))
    sheet = Image.new("RGB", (990, len(sheet_rows) * 250), "#e8e8e8")
    draw = ImageDraw.Draw(sheet)
    for row, (label, images) in enumerate(sheet_rows):
        draw.text((10, row * 250 + 6), label, fill="black")
        for col, image in enumerate(images):
            thumb = image.copy()
            thumb.thumbnail((310, 218))
            sheet.paste(thumb, (10 + col * 330, row * 250 + 28))
    contact = ROOT / "evidence/render-contact-sheet.png"
    sheet.save(contact)
    report.update(result="passed", contact_sheet=str(contact.relative_to(REPO)), contact_sheet_sha256=sha(contact.read_bytes()))
    destination = ROOT / "evidence/render-evidence.json"
    destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "comparisons": 12, "negative_control_detected": True,
                      "evidence": str(destination), "contact_sheet": str(contact)}, indent=2))


if __name__ == "__main__":
    main()
