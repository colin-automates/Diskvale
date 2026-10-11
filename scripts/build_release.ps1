[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $projectRoot
try {
    $sourceCommit = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Release builds require a Git checkout.' }
    $status = git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'Commit source changes before building a release. The checkout must be clean.' }
} finally { Pop-Location }
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
        $currentCommit = git rev-parse HEAD
        if ($LASTEXITCODE -ne 0 -or $currentCommit -ne $sourceCommit) { throw 'Source commit changed during the release build.' }
        $status = git status --porcelain --untracked-files=normal
        if ($LASTEXITCODE -ne 0 -or $status) { throw 'Source changed during the release build; do not publish it.' }
        $binary = Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release/canopy.exe'
        $version = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion
        $rustc = & rustc -Vv
        if ($LASTEXITCODE -ne 0) { throw 'Could not record the Rust toolchain.' }
        $info = [ordered]@{
            product = 'Diskvale'
            version = $version
            source_commit = $sourceCommit
            source_url = "https://github.com/colin-automates/Diskvale/commit/$sourceCommit"
            target = 'x86_64-pc-windows-msvc'
            rustc = $rustc -join "`n"
            static_crt = $true
            executable_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        [IO.File]::WriteAllText((Join-Path (Split-Path $binary) 'BUILD_INFO.json'), ($info | ConvertTo-Json) + "`n", [Text.UTF8Encoding]::new($false))
    } finally { Pop-Location }
} finally {
    if ($null -eq $previousFlags) { Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue }
    else { $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags }
}
