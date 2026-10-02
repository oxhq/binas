# SPDX-License-Identifier: Apache-2.0
"""Windows fresh-process memory observations; stdlib only, no installed deps."""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
from pathlib import Path
import platform
import queue
import statistics
import subprocess
import threading
import time
import zlib


class Counters(c.Structure):
    _fields_ = [('cb', w.DWORD), ('PageFaultCount', w.DWORD)] + [(name, c.c_size_t) for name in
        ['PeakWorkingSetSize', 'WorkingSetSize', 'QuotaPeakPagedPoolUsage', 'QuotaPagedPoolUsage',
         'QuotaPeakNonPagedPoolUsage', 'QuotaNonPagedPoolUsage', 'PagefileUsage', 'PeakPagefileUsage', 'PrivateUsage']]


kernel = c.WinDLL('kernel32', use_last_error=True)
psapi = c.WinDLL('psapi', use_last_error=True)
kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
kernel.OpenProcess.restype = w.HANDLE
kernel.CloseHandle.argtypes = [w.HANDLE]
psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Counters), w.DWORD]


def counters(handle):
    value = Counters(); value.cb = c.sizeof(value)
    if not psapi.GetProcessMemoryInfo(handle, c.byref(value), value.cb):
        raise c.WinError(c.get_last_error())
    return {'working_set_bytes': value.WorkingSetSize, 'peak_working_set_bytes': value.PeakWorkingSetSize,
            'private_bytes': value.PrivateUsage, 'pagefile_bytes': value.PagefileUsage,
            'peak_pagefile_bytes': value.PeakPagefileUsage}


def sample(command, env=None):
    started = time.perf_counter()
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    handle = kernel.OpenProcess(0x410, False, child.pid)
    if not handle: raise c.WinError(c.get_last_error())
    messages = queue.Queue()
    def read():
        for line in child.stdout: messages.put(line)
        messages.put(None)
    threading.Thread(target=read, daemon=True).start()
    observations = []; ready = done = None; baseline = final = None
    try:
        while done is None:
            observations.append({'elapsed_seconds': time.perf_counter() - started, **counters(handle)})
            try: line = messages.get(timeout=0.002)
            except queue.Empty: line = ''
            if line is None: raise RuntimeError(f'child ended before done: {child.stderr.read()}')
            if line:
                if line.strip().endswith('READY'): value = {'ready': True}
                elif line.strip().endswith('DONE'): value = {'done': True}
                else:
                    try: value = json.loads(line)
                    except json.JSONDecodeError: continue
                if value.get('ready'):
                    ready = value; baseline = counters(handle)
                    child.stdin.write('\n'); child.stdin.flush()
                if value.get('done'):
                    done = value; final = counters(handle)
            if time.perf_counter() - started > 90: raise TimeoutError(command)
        child.stdin.write('\n'); child.stdin.flush()
        child.wait(timeout=10)
        stderr = child.stderr.read()
        if child.returncode: raise RuntimeError(f'exit {child.returncode}: {stderr}')
        return {'command': command, 'elapsed_seconds_including_startup': time.perf_counter() - started,
                'baseline': baseline, 'final': final, 'ready': ready, 'result': done, 'stderr': stderr,
                'samples': observations, 'sample_count': len(observations)}
    finally:
        if child.poll() is None: child.kill(); child.wait()
        kernel.CloseHandle(handle)


def pdf(decoded):
    payload = zlib.compress(decoded)
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>',
        f'<< /Filter /FlateDecode /Length {len(payload)} >>\nstream\n'.encode() + payload + b'\nendstream']
    result = b'%PDF-1.4\n'; offsets = []
    for index, obj in enumerate(objects, 1):
        offsets.append(len(result)); result += f'{index} 0 obj\n'.encode() + obj + b'\nendobj\n'
    xref = len(result)
    result += b'xref\n0 5\n0000000000 65535 f \n'
    result += b''.join(f'{offset:010d} 00000 n \n'.encode() for offset in offsets)
    return result + f'trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()


def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--node', default=r'C:\Program Files\nodejs\node.exe')
    parser.add_argument('--repetitions', type=int, default=3)
    args = parser.parse_args()
    root = args.evidence; root.mkdir(parents=True, exist_ok=True)
    fixtures = root / 'fixtures'; fixtures.mkdir(exist_ok=True)
    plain = args.repo / 'crates/binas-pdf/tests/fixtures/retained-ascii/retained-ascii-plain.pdf'
    flate = plain.with_name('retained-ascii-flate.pdf')
    fonted = args.repo / 'crates/binas-pdf/tests/fixtures/fonted-text/fonted-text-plain.pdf'
    prefix = b'BT (ALPHA) Tj ET\n'
    generated = {'input-max': plain.read_bytes().ljust(8 * 1024 * 1024, b' '),
        'input-over': bytes(8 * 1024 * 1024 + 1), 'invalid': b'not a PDF',
        'decode-max': pdf(prefix + b' ' * (16 * 1024 * 1024 - len(prefix))),
        'decode-over': pdf(prefix + b' ' * (16 * 1024 * 1024 + 1 - len(prefix)))}
    for name, data in generated.items(): (fixtures / f'{name}.pdf').write_bytes(data)
    cases = []
    for shape, source in [('plain', plain), ('flate', flate)]:
        for scenario in ['read-retained', 'read-reopen', 'edit-retained', 'edit-reopen']:
            cases.append((f'{shape}-{scenario}', scenario, source))
    for name, scenario, source in [('input-max', 'read-once', fixtures / 'input-max.pdf'),
        ('output-growth', 'output-growth', plain), ('output-over', 'output-refusal', fixtures / 'input-max.pdf'),
        ('decode-max', 'read-once', fixtures / 'decode-max.pdf'), ('input-over', 'input-refusal', fixtures / 'input-over.pdf'),
        ('decode-over', 'decode-refusal', fixtures / 'decode-over.pdf'), ('invalid', 'invalid-refusal', fixtures / 'invalid.pdf'),
        ('live', 'live-refusal', plain), ('font', 'font-refusal', fonted), ('cancel', 'cancel-refusal', plain)]:
        cases.append((name, scenario, source))
    manifest = [{'case': name, 'scenario': scenario, 'fixture': str(source), 'input_bytes': source.stat().st_size,
                 'sha256': sha(source)} for name, scenario, source in cases]
    (root / 'cases.json').write_text(json.dumps(manifest, indent=2))
    environment = {'platform': platform.platform(), 'machine': platform.machine(), 'python': platform.python_version(),
        'node': subprocess.check_output([args.node, '--version'], text=True).strip(), 'cpu_count': os.cpu_count(),
        'git_head': subprocess.check_output(['git', '-C', str(args.repo), 'rev-parse', 'HEAD'], text=True).strip(),
        'git_status': subprocess.check_output(['git', '-C', str(args.repo), 'status', '--short'], text=True),
        'binary_sha256': sha(args.binary), 'wasm_sha256': sha(args.artifacts / 'pkg-node/binas_pdf_wasm_bg.wasm'),
        'source_sha256': {str(path.relative_to(args.repo)): sha(path) for path in
            [args.repo / 'Cargo.lock', args.repo / 'crates/binas-pdf/examples/boundary_peak_memory.rs',
             *sorted((args.repo / 'qa/peak-memory').glob('*.*'))]},
        'engine_binding_source_sha256': {str(path.relative_to(args.repo)): sha(path) for directory in
            ['crates/binas-core/src', 'crates/binas-pdf/src', 'bindings/binas-pdf-wasm/src'] for path in sorted((args.repo / directory).rglob('*.rs'))},
        'sampling_interval_requested_seconds': 0.002, 'repetitions': args.repetitions,
        'probe': 'Windows GetProcessMemoryInfo; parent samples; child waits ready/done; no forced GC'}
    (root / 'environment.json').write_text(json.dumps(environment, indent=2))
    rows = []; failures = []
    for name, scenario, source in cases:
        for runtime in ['native', 'node-wasm']:
            for repetition in range(args.repetitions):
                output = root / f'{runtime}-{name}-{repetition}.pdf'
                command = [str(args.binary), scenario, str(source), str(output)] if runtime == 'native' else [args.node,
                    str(args.repo / 'qa/peak-memory/node.mjs'), str(args.artifacts / 'pkg-node/binas_pdf_wasm.js'),
                    scenario, str(source), str(output)]
                try:
                    row = sample(command); row.update(runtime=runtime, case=name, repetition=repetition)
                    if output.exists(): row['output_sha256'] = sha(output)
                    rows.append(row)
                except Exception as error:
                    failures.append({'runtime': runtime, 'case': name, 'repetition': repetition, 'error': str(error)})
                (root / 'samples.json').write_text(json.dumps({'observations': rows, 'failures': failures}, indent=2))
            print(f'{runtime}/{name}: {sum(r["runtime"] == runtime and r["case"] == name for r in rows)}/{args.repetitions}', flush=True)
    summary = []
    for name, scenario, source in cases:
        for runtime in ['native', 'node-wasm']:
            selected = [row for row in rows if row['case'] == name and row['runtime'] == runtime]
            if not selected: continue
            def stats(values): return {'min': min(values), 'median': statistics.median(values), 'max': max(values)}
            item = {'case': name, 'runtime': runtime, 'successful_probes': len(selected), 'planned_probes': args.repetitions,
                'outcomes': [r['result']['error'] for r in selected], 'input_bytes': source.stat().st_size,
                'peak_working_set_bytes': stats([r['final']['peak_working_set_bytes'] for r in selected]),
                'baseline_working_set_bytes': stats([r['baseline']['working_set_bytes'] for r in selected]),
                'sampled_private_bytes_max': stats([max(p['private_bytes'] for p in r['samples']) for r in selected]),
                'sampled_working_set_bytes_max': stats([max(p['working_set_bytes'] for p in r['samples']) for r in selected])}
            if runtime == 'node-wasm':
                item['wasm_linear_high_water_bytes'] = stats([r['result']['final']['wasm_linear_bytes'] for r in selected])
                item['js_heap_used_phase_max_bytes'] = stats([max(p['heapUsed'] for p in r['result']['phases']) for r in selected])
            summary.append(item)
    for shape in ['plain', 'flate']:
        matches = [r['output_sha256'] for r in rows if r['case'].startswith(shape + '-edit')]
        if len(set(matches)) != 1: failures.append({'parity': shape, 'hashes': matches})
    growing = [r['output_sha256'] for r in rows if r['case'] == 'output-growth']
    if len(set(growing)) != 1: failures.append({'parity': 'output-growth', 'hashes': growing})
    report = {'summary': summary, 'planned_probes': len(cases) * 2 * args.repetitions,
        'successful_probes': len(rows), 'failures': failures, 'edit_retained_reopen_native_wasm_byte_parity': not failures}
    (root / 'summary.json').write_text(json.dumps(report, indent=2))
    if failures: raise SystemExit(f'{len(failures)} failed probes; see summary.json')


if __name__ == '__main__': main()
