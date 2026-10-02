"""Bounded public nontext gate. Original Binas QA, Apache-2.0.

Reuse frozen preservation fixtures and unchanged independent oracles. Stage all
PDFs, renderings and receipts outside the checkout. No fixture generation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

REPO = Path(__file__).resolve().parents[2]
OPERATIONS = ["reorder", "geometry", "checkbox", "flatten", "text_field",
              "duplicate", "delete", "lossless_images", "import"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    if not __debug__ or os.environ.get("PYTHONOPTIMIZE"):
        raise RuntimeError("Independent assert-based oracles require unoptimized Python")
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--pdftoppm", type=Path, required=True)
    p.add_argument("--consumer-manifest", type=Path,
                   help="External consumer Cargo.toml importing extracted binas-core/PDF")
    args = p.parse_args()
    def git_identity():
        return {"head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
                "status": subprocess.check_output(["git", "status", "--porcelain=v1", "--untracked-files=all"], cwd=REPO, text=True)}
    source_initial = git_identity()
    output = args.output.resolve()
    if output == REPO or REPO in output.parents:
        raise ValueError("Evidence output must be outside checkout")
    output.mkdir(parents=True, exist_ok=True)
    stage = output / "preservation-workspace"
    qa = stage / "qa/preservation"
    qa.mkdir(parents=True, exist_ok=True)
    files = []
    for rel in ["qa/nontext-boundary/run.py", "qa/nontext-boundary/oracle.py",
                "crates/binas-pdf/tests/nontext_boundary.rs"]:
        files.append({"path": rel, "sha256": sha(REPO / rel)})
    for path in sorted((REPO / "qa/preservation").glob("*.py")):
        dest = qa / path.name
        shutil.copyfile(path, dest)
        files.append({"path": str(path.relative_to(REPO)), "sha256": sha(path)})
    for name in ["LICENSE", "manifest.json", "requirements.txt"]:
        shutil.copyfile(REPO / "qa/preservation" / name, qa / name)
        files.append({"path": f"qa/preservation/{name}", "sha256": sha(REPO / "qa/preservation" / name)})
    fixtures = qa / "fixtures"
    fixtures.mkdir(exist_ok=True)
    original = REPO / "qa/preservation"
    manifest = json.loads((original / "manifest.json").read_text())
    expected = {e["path"]: e["sha256"] for e in manifest["required"]}
    for path in sorted((original / "fixtures").glob("*.provenance.json")):
        record = json.loads(path.read_text(encoding="utf-8"))
        if "path" in record:
            expected[record["path"]] = record["sha256"]
        elif "source_path" in record:
            expected[record["source_path"]] = record["source_sha256"]
        shutil.copyfile(path, fixtures / path.name)
        files.append({"path": str(path.relative_to(REPO)), "sha256": sha(path)})
    for rel, frozen in sorted(expected.items()):
        path = original / rel
        if sha(path) != frozen:
            raise AssertionError(f"Frozen fixture mismatch: {path}")
        shutil.copyfile(path, qa / rel)
        files.append({"path": str(path.relative_to(REPO)), "sha256": frozen})
    for rel in ["crates/binas-pdf/tests/fixtures/preservation/manifest.json",
                "crates/binas-pdf/tests/fixtures/preservation/type3-shared.pdf",
                "crates/binas-pdf/tests/fixtures/preservation/type3-separate.pdf"]:
        dest = stage / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(REPO / rel, dest)
        files.append({"path": rel, "sha256": sha(REPO / rel)})
    for operation in OPERATIONS:
        rel = f"crates/binas-pdf/examples/preservation_{operation}.rs"
        dest = stage / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(REPO / rel, dest)
        files.append({"path": rel, "sha256": sha(REPO / rel)})
    evidence = qa / "evidence"
    evidence.mkdir(exist_ok=True)
    env = os.environ.copy()
    env["BINAS_PRESERVATION_OUTPUT_DIR"] = str(evidence)
    env["BINAS_PRESERVATION_INPUT_DIR"] = str(REPO / "qa/preservation")
    env["BINAS_IMPORT_INPUT_DIR"] = str(REPO / "crates/binas-pdf/tests/fixtures/preservation")
    env["BINAS_NONTEXT_OUTPUT_DIR"] = str(output / "lifecycle")
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env.pop("PYTHONOPTIMIZE", None)
    commands = []
    selection = (["--manifest-path", str(args.consumer_manifest.resolve())]
                 if args.consumer_manifest else ["-p", "binas-pdf"])

    def run(name, command):
        result = subprocess.run(command, cwd=REPO, env=env, capture_output=True,
                                text=True, encoding="utf-8", errors="replace")
        log = output / f"{name}.log"
        log.write_text(result.stdout + result.stderr, encoding="utf-8")
        commands.append({"name": name, "command": command, "exit_code": result.returncode,
                         "log": str(log), "log_sha256": sha(log)})
        print(f"{name}: exit {result.returncode}", flush=True)
        if result.returncode:
            raise RuntimeError(f"{name} failed: {log}")

    try:
        for operation in OPERATIONS:
            run(f"example-{operation}", ["cargo", "run", "--offline", "--locked", *selection,
                                         "--example", f"preservation_{operation}"])
        run("public-nontext", ["cargo", "test", "--offline", "--locked", *selection,
                              "--test", "nontext_boundary", "--", "--nocapture"])
        for prefix, script in [("reordered", "oracle"), ("alpha", "alpha_oracle")]:
            for mode in ["incremental", "compact"]:
                run(f"oracle-{prefix}-{mode}", [sys.executable, str(qa / f"{script}.py"),
                    "verify", "--candidate", str(evidence / f"{prefix}-{mode}.pdf")])
        run("render-reorder", [sys.executable, str(qa / "render_oracle.py"),
                               "--pdftoppm", str(args.pdftoppm)])
        for operation in ["geometry", "checkbox", "flatten", "text_field", "duplicate",
                          "delete", "lossless_image"]:
            run(f"oracle-{operation}", [sys.executable, str(qa / f"{operation}_oracle.py"),
                                         "--pdftoppm", str(args.pdftoppm)])
        run("oracle-import", [sys.executable, str(qa / "import_oracle.py"), "verify"])
        run("render-import", [sys.executable, str(qa / "import_render.py"),
                              "--pdftoppm", str(args.pdftoppm)])
        run("oracle-lifecycle", [sys.executable, str(REPO / "qa/nontext-boundary/oracle.py"),
                                  "--output", str(output / "lifecycle"),
                                  "--pdftoppm", str(args.pdftoppm)])
        for entry in files:
            if sha(REPO / entry["path"]) != entry["sha256"]:
                raise AssertionError(f"Input changed during gate: {entry['path']}")
        if git_identity()["head"] != source_initial["head"]:
            raise AssertionError("Source HEAD changed during gate")
        status = "passed"
    except Exception:
        status = "failed"
        raise
    finally:
        report = {"schema": 1, "status": status, "scope": "Frozen synthetic nontext cohort only",
                  "source_initial": source_initial, "source_final": git_identity(),
                  "source_clean_initial": not source_initial["status"],
                  "consumer_manifest": str(args.consumer_manifest) if args.consumer_manifest else None,
                  "python": sys.version, "inputs": files, "commands": commands,
                  "renderer_sha256": sha(args.pdftoppm),
                  "artifacts": [{"path": str(p.relative_to(output)), "bytes": p.stat().st_size,
                                 "sha256": sha(p)} for p in sorted(output.rglob("*"))
                                if p.is_file() and p.name != "summary.json"]}
        (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
