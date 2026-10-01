param([switch]$Dev)
$ErrorActionPreference = 'Stop'
$baRoot = Split-Path $PSScriptRoot -Parent
$baLocalTools = Join-Path (Split-Path $baRoot -Parent) '.tools'
$baOldCargo = $env:CARGO_HOME
$baOldRustup = $env:RUSTUP_HOME
$baOldPath = $env:PATH
try {
    if (Test-Path -LiteralPath (Join-Path $baLocalTools 'cargo/bin/cargo.exe')) {
        $env:CARGO_HOME = Join-Path $baLocalTools 'cargo'
        $env:RUSTUP_HOME = Join-Path $baLocalTools 'rustup'
        $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
    }
    Push-Location (Join-Path $baRoot 'wasm')
    try {
        wasm-pack build --target web --out-dir ../public/wasm --locked
        if ($LASTEXITCODE -ne 0) { throw 'WASM build failed' }
    } finally { Pop-Location }
    Push-Location $baRoot
    try {
        if ($Dev) { pnpm desktop:dev } else { pnpm desktop:build }
        if ($LASTEXITCODE -ne 0) { throw 'Desktop build failed' }
    } finally { Pop-Location }
} finally {
    $env:CARGO_HOME = $baOldCargo
    $env:RUSTUP_HOME = $baOldRustup
    $env:PATH = $baOldPath
}
