# SPDX-License-Identifier: Apache-2.0
"""Summarize all raw samples with nearest-rank p50/p95; no dependencies."""
import hashlib
import json
import math
from pathlib import Path
import sys

evidence = Path(sys.argv[1])
report = json.loads((evidence / "samples.json").read_text())


def summary(values, divisor=1):
    values = sorted(value / divisor / 1000 for value in values)
    return {"n": len(values), "unit": "microseconds", "min": min(values),
            "p50": values[math.ceil(len(values) * .50) - 1],
            "p95": values[math.ceil(len(values) * .95) - 1], "max": max(values)}


cases = {}
for case in report["cases"]:
    phases = case["samples"][0]["phases"].keys()
    cases[case["fixture"]] = {
        key: summary([sample["phases"][key] for sample in case["samples"]],
                     report["reads_per_batch"] if "read_batch" in key else 1)
        for key in phases}
batches = report["raw_flate_batch_samples"]
cases["raw-flate-batches"] = {
    key: summary([sample[key] for sample in batches])
    for key in batches[0] if key != "index"}
(evidence / "summary.json").write_text(json.dumps(cases, indent=2) + "\n")
manifest = {"schema": 1, "files": []}
for path in sorted(evidence.rglob("*")):
    if path.is_file() and "target" not in path.relative_to(evidence).parts and path.name not in {"manifest.json", "summary.log"}:
        data = path.read_bytes()
        manifest["files"].append({"path": path.relative_to(evidence).as_posix(),
                                  "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
(evidence / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps(cases, indent=2))
