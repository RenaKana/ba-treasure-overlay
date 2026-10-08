param([string]$OutputDirectory, [switch]$SkipBuild, [string]$ConfigPath)
$ErrorActionPreference = 'Stop'
$baRepo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $baRepo ('.artifacts/partial-recognition/run-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$baOutput = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $baOutput) { throw 'Use a new output directory to preserve previous runs.' }
New-Item -ItemType Directory -Path $baOutput | Out-Null
$baOldCargo = $env:CARGO_HOME; $baOldRustup = $env:RUSTUP_HOME; $baOldPath = $env:PATH
try {
    $baTools = Join-Path $baRepo '.tools'
    if (Test-Path -LiteralPath (Join-Path $baTools 'cargo/bin/cargo.exe')) {
        $env:CARGO_HOME = Join-Path $baTools 'cargo'; $env:RUSTUP_HOME = Join-Path $baTools 'rustup'
        $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
    }
    $baCargo = Join-Path $baRepo 'app/src-tauri/Cargo.toml'
    if (-not $SkipBuild) {
        & cargo build --manifest-path $baCargo --locked --release --example partial-recognition --features partial-recognition-experiment *> (Join-Path $baOutput 'build.log')
        if ($LASTEXITCODE -ne 0) { throw "Build failed; see $baOutput/build.log" }
    }
    $baExe = Join-Path $baRepo 'app/src-tauri/target/release/examples/partial-recognition.exe'
    $baManifest = Join-Path $PSScriptRoot 'manifest.json'
    $baConfig = if ($ConfigPath) { [IO.Path]::GetFullPath($ConfigPath) } else { Join-Path $PSScriptRoot 'config.json' }
    & $baExe --manifest $baManifest --config $baConfig --output (Join-Path $baOutput 'tuning') --split tuning *> (Join-Path $baOutput 'tuning.log')
    if ($LASTEXITCODE -ne 0) { throw "Tuning replay failed; see $baOutput/tuning.log" }
    & $baExe --manifest $baManifest --config $baConfig --output (Join-Path $baOutput 'validation') --split validation --freeze-record (Join-Path $baOutput 'tuning/freeze.json') *> (Join-Path $baOutput 'validation.log')
    if ($LASTEXITCODE -ne 0) { throw "Validation replay failed; see $baOutput/validation.log" }
    & node (Join-Path $PSScriptRoot 'summarize.mjs') $baOutput
    if ($LASTEXITCODE -ne 0) { throw 'Report aggregation failed.' }
    & node (Join-Path $PSScriptRoot 'check-results.mjs') $baOutput
    if ($LASTEXITCODE -ne 0) { throw 'Saved evidence integrity check failed.' }
    $baEvidenceFiles = @($baExe,$baCargo,(Join-Path $baRepo 'app/src-tauri/Cargo.lock'),(Join-Path $baRepo 'app/src-tauri/examples/partial_recognition.rs'))
    $baEvidenceFiles += @(Get-ChildItem -LiteralPath $PSScriptRoot -File | ForEach-Object FullName)
    if ($baConfig -notin $baEvidenceFiles) { $baEvidenceFiles += $baConfig }
    $baEvidenceFiles += @(Get-ChildItem -LiteralPath (Join-Path $baRepo 'app/src-tauri/src') -File | ForEach-Object FullName)
    $baManifestData = Get-Content -Raw -LiteralPath $baManifest | ConvertFrom-Json
    $baEvidenceFiles += @($baManifestData.frames.source | Select-Object -Unique | ForEach-Object { [IO.Path]::GetFullPath((Join-Path $PSScriptRoot $_)) })
    $baEvidenceFiles | Get-FileHash -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $baOutput 'sha256.json')
    Write-Output (Join-Path $baOutput 'index.html')
} finally {
    $env:CARGO_HOME = $baOldCargo; $env:RUSTUP_HOME = $baOldRustup; $env:PATH = $baOldPath
}
