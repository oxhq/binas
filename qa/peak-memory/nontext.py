# SPDX-License-Identifier: Apache-2.0
"""Measure B's existing public nontext tests; memory is per entire workflow."""
import argparse
import json
import os
from pathlib import Path
import subprocess
from run import sample, sha

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--repo', type=Path, required=True)
parser.add_argument('--evidence', type=Path, required=True)
parser.add_argument('--repetitions', type=int, default=3)
parser.add_argument('--page-binary', type=Path, required=True)
parser.add_argument('--page-golden', type=Path, required=True)
args = parser.parse_args()
args.evidence.mkdir(parents=True, exist_ok=True)
tests = [line.split(':')[0] for line in subprocess.check_output([str(args.binary), '--list'], text=True).splitlines() if line.endswith(': test')]
rows = []; failures = []
env = dict(os.environ, BINAS_NONTEXT_MEMORY_HOLD='1', BINAS_PRESERVATION_INPUT_DIR=str(args.repo / 'qa/preservation'))
for test in tests:
    for repetition in range(args.repetitions):
        try:
            row = sample([str(args.binary), test, '--exact', '--nocapture', '--test-threads=1'], env)
            rows.append({'case': test, 'repetition': repetition, **row})
        except Exception as error: failures.append({'case': test, 'repetition': repetition, 'error': str(error)})
    print(f'{test}: {sum(row["case"] == test for row in rows)}/{args.repetitions}', flush=True)
page_fixture = args.repo / 'crates/binas-pdf/tests/fixtures/preservation/type3-shared.pdf'
for repetition in range(args.repetitions):
    output = args.evidence / f'page-import-{repetition}.pdf'
    try:
        row = sample([str(args.page_binary), 'page-import', str(page_fixture), str(output)])
        if output.read_bytes() != args.page_golden.read_bytes(): raise AssertionError('page-import differs from B independent-oracle candidate')
        rows.append({'case': 'page-import', 'repetition': repetition, 'output_sha256': sha(output), **row})
    except Exception as error: failures.append({'case': 'page-import', 'repetition': repetition, 'error': str(error)})
print(f'page-import: {sum(row["case"] == "page-import" for row in rows)}/{args.repetitions}', flush=True)
report = {'planned_probes': (len(tests) + 1) * args.repetitions, 'successful_probes': len(rows), 'binary_sha256': sha(args.binary),
    'page_binary_sha256': sha(args.page_binary), 'page_fixture_sha256': sha(page_fixture), 'page_golden_sha256': sha(args.page_golden),
    'git_head': subprocess.check_output(['git','-C',str(args.repo),'rev-parse','HEAD'],text=True).strip(),
    'method': 'fresh Windows native process, OS lifetime PeakWorkingSet; five debug test workflows plus release page-import example; per entire public workflow, no forced GC or binary-size-normalized comparison',
    'observations': rows, 'failures': failures}
(args.evidence / 'nontext.json').write_text(json.dumps(report, indent=2))
if failures: raise SystemExit(f'{len(failures)} failed probes')
