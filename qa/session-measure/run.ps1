# SPDX-License-Identifier: Apache-2.0
param(
    [Parameter(Mandatory=$true)][string]$Evidence,
    [string]$Python = 'C:\Users\garae\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$Evidence = [System.IO.Path]::GetFullPath($Evidence)
New-Item -ItemType Directory -Force -Path $Evidence | Out-Null
$environment = [ordered]@{
    captured_utc = (Get-Date).ToUniversalTime().ToString('o')
    git_head = (& git -C $repo rev-parse HEAD)
    git_status = @(& git -C $repo status --short)
    rustc = @(& rustc --version --verbose)
    cargo = (& cargo --version)
    operating_system = (Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,OSArchitecture)
    cpu = @(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors)
    physical_memory_bytes = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
    power_plan = @(& powercfg /getactivescheme)
    build_command = 'cargo build --offline --locked --release -p binas-pdf --example session_measure --target-dir EVIDENCE/target'
    source_hash = (Get-FileHash (Join-Path $repo 'crates/binas-pdf/examples/session_measure.rs') -Algorithm SHA256).Hash
    cargo_lock_hash = (Get-FileHash (Join-Path $repo 'Cargo.lock') -Algorithm SHA256).Hash
    source_fixture_hashes = @(Get-ChildItem (Join-Path $repo 'crates/binas-pdf/tests/fixtures/retained-ascii/retained-ascii-*.pdf'),(Join-Path $repo 'crates/binas-pdf/tests/fixtures/fonted-text/fonted-text-plain.pdf'),(Join-Path $repo 'crates/binas-pdf/tests/fixtures/fonted-text/fonted-text-flate.pdf') | Get-FileHash -Algorithm SHA256 | Select-Object Path,Hash)
}
$environment | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 (Join-Path $Evidence 'environment.json')
Push-Location $repo
try {
    & cargo build --offline --locked --release -p binas-pdf --example session_measure --target-dir (Join-Path $Evidence 'target') *> (Join-Path $Evidence 'build.log')
    if ($LASTEXITCODE -ne 0) { throw 'release build failed; see build.log' }
    $binary = Join-Path $Evidence 'target/release/examples/session_measure.exe'
    & $binary $repo $Evidence *> (Join-Path $Evidence 'run.log')
    if ($LASTEXITCODE -ne 0) { throw 'correctness/timing failed; see run.log' }
    # Independent existing P3a parser, geometry, preservation and untouched-render checks.
    foreach ($shape in @('plain','flate')) {
        $source = Join-Path $repo "crates/binas-pdf/tests/fixtures/retained-ascii/retained-ascii-$shape.pdf"
        foreach ($mode in @('incremental','compact','multi-retained-compact','multi-reopen-compact')) {
            $output = Join-Path $Evidence "retained-ascii-$shape-$mode.pdf"
            $writer = if ($mode -eq 'incremental') { 'Incremental' } else { 'Compact' }
            & $Python -B qa/retained-ascii/oracle.py verify $source $output $writer (Join-Path $Evidence "oracle-$shape-$mode.json") *> (Join-Path $Evidence "oracle-$shape-$mode.log")
            if ($LASTEXITCODE -ne 0) { throw "independent oracle failed: $shape $mode" }
        }
    }
    Get-FileHash $binary -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Evidence 'binary.json')
    & $Python -B qa/session-measure/summarize.py $Evidence *> (Join-Path $Evidence 'summary.log')
    if ($LASTEXITCODE -ne 0) { throw 'summary failed' }
} finally { Pop-Location }
Write-Output "P6 correctness, independent oracles and samples complete: $Evidence"
