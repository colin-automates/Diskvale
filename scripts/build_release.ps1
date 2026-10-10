[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
if ($env:RUSTFLAGS -and -not $previousFlags) {
    throw 'For custom release flags, use CARGO_ENCODED_RUSTFLAGS instead of RUSTFLAGS.'
}
$flags = @()
if ($previousFlags) { $flags += $previousFlags.Split([char]31) }
# Rust error locations can contain absolute compiler input paths. Remap them so
# distributed binaries do not disclose the build machine's profile or checkout.
if ($env:USERPROFILE) { $flags += "--remap-path-prefix=$($env:USERPROFILE)=build-home" }
if ($env:CARGO_HOME) { $flags += "--remap-path-prefix=$($env:CARGO_HOME)=cargo-home" }
if ($env:RUSTUP_HOME) { $flags += "--remap-path-prefix=$($env:RUSTUP_HOME)=rustup-home" }
$flags += "--remap-path-prefix=$projectRoot=Diskvale"
$flags += '-C', 'target-feature=+crt-static'
try {
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
    Push-Location $projectRoot
    try {
        $cargoExecutable = if ($env:CARGO) { $env:CARGO } else { 'cargo' }
        # The explicit target applies crt-static only to application crates,
        # leaving host build scripts and proc macros loadable by Cargo.
        & $cargoExecutable build --release --locked --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
} finally {
    if ($null -eq $previousFlags) { Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue }
    else { $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags }
}
