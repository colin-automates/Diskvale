[CmdletBinding()]
param(
    [string]$Executable = '',
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $Executable) { $Executable = Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release/canopy.exe' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot 'artifacts/release' }
$Executable = [IO.Path]::GetFullPath($Executable)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$cargo = Get-Content -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -Raw
$versionMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) { throw 'Cargo package version was not found.' }
$version = $versionMatch.Groups[1].Value
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw 'Release executable is missing. Run cargo build --release --locked first.'
}
$binaryVersion = (Get-Item -LiteralPath $Executable).VersionInfo.ProductVersion
if ($binaryVersion -ne $version) { throw "Executable version $binaryVersion differs from Cargo version $version." }

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$staging = Join-Path $OutputDirectory 'Diskvale'
New-Item -ItemType Directory -Path $staging -Force | Out-Null
foreach ($name in @('LICENSE', 'THIRD_PARTY_NOTICES.txt')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination (Join-Path $staging $name) -Force
}
Copy-Item -LiteralPath $Executable -Destination (Join-Path $staging 'Diskvale.exe') -Force
$quickStart = @"
Diskvale $version - Disk space, clearly.

1. Extract this ZIP into a folder.
2. Open Diskvale.exe and approve the Windows administrator prompt.
3. Choose an NTFS drive and click Scan. Use Rescan to refresh the results.

Windows x64 and administrator access are required. No installation is needed.
This release is unsigned and uses an experimental NTFS reader.

Map: explore storage visually. Double-click folders; Backspace goes up.
Files: find the largest files. Types: see totals by extension.
Search example: ext:mp4 >1GB. Ctrl+K focuses search.

Delete actions ask you to confirm the exact target. If recycling is refused,
permanent deletion requires a separate confirmation and cannot be undone.

Full source, documentation, updates and demo:
https://github.com/colin-automates/Diskvale

Diskvale is MIT licensed. See LICENSE and THIRD_PARTY_NOTICES.txt.
"@
[IO.File]::WriteAllText((Join-Path $staging 'README.txt'), $quickStart + "`r`n", [Text.UTF8Encoding]::new($false))
$zip = Join-Path $OutputDirectory 'Diskvale-windows-x64.zip'
# An explicit list keeps unrelated files from an older staging folder out.
$packageFiles = @('Diskvale.exe', 'README.txt', 'LICENSE', 'THIRD_PARTY_NOTICES.txt') | ForEach-Object { Join-Path $staging $_ }
Compress-Archive -LiteralPath $packageFiles -DestinationPath $zip -Force
Copy-Item -LiteralPath (Join-Path $staging 'Diskvale.exe') -Destination (Join-Path $OutputDirectory 'Diskvale.exe') -Force
Copy-Item -LiteralPath (Join-Path $staging 'THIRD_PARTY_NOTICES.txt') -Destination (Join-Path $OutputDirectory 'THIRD_PARTY_NOTICES.txt') -Force
$demo = Join-Path $projectRoot 'docs/media/diskvale-demo.mp4'
if (Test-Path -LiteralPath $demo) { Copy-Item -LiteralPath $demo -Destination (Join-Path $OutputDirectory 'Diskvale-Demo.mp4') -Force }
$downloads = @('Diskvale.exe', 'Diskvale-windows-x64.zip', 'THIRD_PARTY_NOTICES.txt')
if (Test-Path -LiteralPath (Join-Path $OutputDirectory 'Diskvale-Demo.mp4')) { $downloads += 'Diskvale-Demo.mp4' }
$hashes = foreach ($name in $downloads) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $OutputDirectory $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $name"
}
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'SHA256SUMS.txt'), ($hashes -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
Write-Output "Packaged Diskvale $version in $OutputDirectory"
Write-Output $hashes
