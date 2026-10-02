#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Local artifact qualification; never publishes, installs tools, or uses credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time
import tomllib


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--target', required=True, type=Path)
    parser.add_argument('--oxpdf-source', type=Path, help='P1 facade root with rust/src and rust/tests')
    parser.add_argument('--msrv', default='1.88.0')
    args = parser.parse_args()
    source, out, target = (p.resolve() for p in (args.source, args.out, args.target))
    if out.exists():
        raise SystemExit('Use a fresh --out directory; prior evidence is preserved')
    if out == source or source in out.parents or out in source.parents:
        raise SystemExit('--out must be separate from --source')
    out.mkdir(parents=True)
    version = tomllib.loads((source / 'Cargo.toml').read_text(encoding='utf-8-sig'))['workspace']['package']['version']
    candidate_lock = digest(source / 'Cargo.lock')
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true', PYTHONDONTWRITEBYTECODE='1')
    records = []

    def run(name, command, cwd, allow_failure=False):
        started = time.time()
        stdout, stderr = out / (name + '.stdout.log'), out / (name + '.stderr.log')
        with stdout.open('wb') as so, stderr.open('wb') as se:
            result = subprocess.run(command, cwd=cwd, env=env, stdout=so, stderr=se)
        record = dict(name=name, command=command, cwd=str(cwd), started_unix=started,
                      seconds=time.time()-started, exit=result.returncode,
                      stdout=str(stdout), stdout_sha256=digest(stdout),
                      stderr=str(stderr), stderr_sha256=digest(stderr))
        records.append(record)
        (out / 'commands.json').write_text(json.dumps(records, indent=2))
        print(f'{name}: exit={result.returncode}', flush=True)
        if result.returncode and not allow_failure:
            raise RuntimeError(f'{name} failed: see {stderr}')
        return stdout

    cargo = shutil.which('cargo')
    if not cargo:
        raise SystemExit('Installed cargo is required')
    head = run('source-head', ['git', 'rev-parse', 'HEAD'], source).read_text().strip()
    if subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain']).strip():
        raise SystemExit('Commit the exact candidate before packaging')
    staging = out / 'staging'
    run('stage-clone', ['git', 'clone', '--local', '--no-hardlinks', str(source), str(staging)], out)
    tracked = subprocess.check_output(['git', '-C', str(source), 'ls-files', '-z']).decode().split('\0')
    source_manifest = [dict(path=p, sha256=digest(source / p)) for p in filter(None, tracked) if (source / p).is_file()]
    (out / 'source-manifest.json').write_text(json.dumps(source_manifest, indent=2))
    for path in filter(None, tracked):
        if (source / path).is_file():
            shutil.copyfile(source / path, staging / path)
    # Copying exact bytes over a Windows checkout can leave libgit2 stat/EOL
    # entries dirty even when normalized blobs are identical. Refresh only the
    # tracked disposable clone; no staged source delta is allowed.
    run('stage-index-refresh', ['git', 'add', '-u'], staging)
    assert not subprocess.check_output(['git', '-C', str(staging), 'diff', '--cached', '--name-only']).strip()
    assert not subprocess.check_output(['git', '-C', str(staging), 'status', '--porcelain']).strip()
    extracted = out / 'extracted'
    archives = out / 'archives'
    extracted.mkdir()
    archives.mkdir()
    artifacts = {}
    archive_manifest = []
    patch = []
    for name in ('binas-core', 'binas-pdf', 'binas-cli'):
        command = [cargo, 'package', '-p', name, '--offline']
        if name == 'binas-core':
            command.append('--locked')
        else:
            # Config patches are unused by source path dependencies and can alter
            # resolver metadata. Only this disposable staging lock may change.
            command += ['--allow-dirty'] + patch
        run(name + '-package-list', command + ['--list'], staging)
        run(name + '-package', command, staging)  # Default verification; no --no-verify.
        archive = archives / f'{name}-{version}.crate'
        shutil.copyfile(target / 'package' / archive.name, archive)
        prefix = f'{name}-{version}'
        with tarfile.open(archive) as tar:
            names = tar.getnames()
            for member in tar.getmembers():
                p = Path(member.name)
                if p.is_absolute() or '..' in p.parts or p.parts[0] != prefix or member.issym() or member.islnk():
                    raise RuntimeError('Unsafe archive member: ' + member.name)
            if any('/tests/' in n or n.endswith('.pdf') or '/target/' in n for n in names):
                raise RuntimeError('Unexpected fixtures/tests/build in release archive')
            tar.extractall(extracted, filter='data')
        artifact = extracted / prefix
        artifacts[name] = artifact
        if digest(artifact / 'LICENSE') != digest(staging / 'crates' / name / 'LICENSE'):
            raise RuntimeError('License mismatch')
        for f in (source / 'crates' / name / 'src').rglob('*'):
            if f.is_file():
                relative = f.relative_to(source / 'crates' / name)
                if digest(f) != digest(artifact / relative):
                    raise RuntimeError('Packaged source differs: ' + str(relative))
        archive_manifest.append(dict(name=name, version=version, path=str(archive), bytes=archive.stat().st_size,
                                     sha256=digest(archive), files=names, default_package_verification=True))
        if name != 'binas-cli':
            patch += ['--config', f'patch.crates-io.{name}.path="{artifact.as_posix()}"']
    (out / 'artifact-manifest.json').write_text(json.dumps(archive_manifest, indent=2))
    consumer = out / 'consumer'
    shutil.copytree(source / 'qa/release-candidate/consumer', consumer)
    text = (consumer / 'Cargo.toml.template').read_text(encoding='utf-8-sig')
    for token, value in [('VERSION', version), ('PDF_ARTIFACT', artifacts['binas-pdf'].as_posix()),
                         ('CORE_ARTIFACT', artifacts['binas-core'].as_posix())]:
        text = text.replace('__' + token + '__', value)
    (consumer / 'Cargo.toml').write_text(text)
    run('consumer-lock', [cargo, 'generate-lockfile', '--offline'], consumer)

    def metadata(name, directory):
        path = run(name, [cargo, 'metadata', '--format-version', '1', '--offline', '--locked'], directory)
        data = json.loads(path.read_text())
        for key in ('binas-core', 'binas-pdf'):
            packages = [p for p in data['packages'] if p['name'] == key]
            assert len(packages) == 1
            p = packages[0]
            assert p['version'] == version and p['source'] is None
            assert Path(p['manifest_path']).resolve() == (artifacts[key] / 'Cargo.toml').resolve()
        return data

    metadata('consumer-metadata', consumer)
    run('consumer-tree', [cargo, 'tree', '--offline', '--locked'], consumer)
    run('consumer-check', [cargo, 'check', '--all-targets', '--offline', '--locked'], consumer)
    run('consumer-run', [cargo, 'run', '--offline', '--locked', '--', str(out)], consumer)
    if args.msrv:
        run('consumer-msrv', [cargo, '+' + args.msrv, 'check', '--all-targets', '--offline', '--locked'], consumer)
    # The executable is built from the already extracted CLI artifact using
    # artifact-only sibling overrides and its own lock, never from the checkout.
    cli = artifacts['binas-cli']
    run('cli-artifact-lock', [cargo, 'generate-lockfile', '--offline'] + patch, cli)
    run('cli-artifact-run', [cargo, 'run', '--offline', '--locked'] + patch +
        ['--', 'inspect', str(out / 'consumer-compact.pdf'), '--json'], cli)
    if args.msrv:
        run('cli-artifact-msrv', [cargo, '+' + args.msrv, 'check', '--all-targets', '--offline', '--locked'] + patch, cli)
    if args.oxpdf_source:
        ox_source = args.oxpdf_source.resolve()
        ox = out / 'oxpdf-minimal'
        (ox / 'src').mkdir(parents=True)
        (ox / 'tests').mkdir()
        shutil.copyfile(ox_source / 'rust/src/lib.rs', ox / 'src/lib.rs')
        shutil.copyfile(ox_source / 'rust/tests/page_text_contract.rs', ox / 'tests/page_text_contract.rs')
        shutil.copytree(ox_source / 'rust/tests/support', ox / 'tests/support')
        text = (ox_source / 'rust/Cargo.toml').read_text(encoding='utf-8-sig').replace('binas-pdf = "0.1.1"', f'binas-pdf = "={version}"')
        text += '\n[workspace]\n\n[patch.crates-io]\n'
        for key in ('binas-core', 'binas-pdf'):
            text += f'{key} = {{ path = "{artifacts[key].as_posix()}" }}\n'
        (ox / 'Cargo.toml').write_text(text)
        run('oxpdf-lock', [cargo, 'generate-lockfile', '--offline'], ox)
        metadata('oxpdf-metadata', ox)
        run('oxpdf-page-contract', [cargo, 'test', '--test', 'page_text_contract', '--offline', '--locked'], ox)
        (out / 'oxpdf-source.json').write_text(json.dumps(dict(origin=str(ox_source), files=[
            dict(path='rust/src/lib.rs', sha256=digest(ox / 'src/lib.rs')),
            dict(path='rust/tests/page_text_contract.rs', sha256=digest(ox / 'tests/page_text_contract.rs'))]), indent=2))
    assert digest(source / 'Cargo.lock') == candidate_lock, 'Candidate lock changed'
    for entry in source_manifest:
        assert digest(source / entry['path']) == entry['sha256'], 'Candidate source changed: ' + entry['path']
    assert subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD']).decode().strip() == head
    assert not subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain']).strip()
    result = dict(status='passed', source=str(source), source_head=head, version=version,
                  candidate_lock_sha256=candidate_lock, staging_lock_sha256=digest(staging / 'Cargo.lock'),
                  overrides='unpublished siblings resolved exclusively from extracted archives; staging resolver lock may change',
                  default_package_verification=True, no_verify=False, public_registry_install=False,
                  msrv=args.msrv, oxpdf_minimal=bool(args.oxpdf_source))
    (out / 'result.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
