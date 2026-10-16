[CmdletBinding()]
param(
    [string]$Executable = '',
    [string]$OutputDirectory = '',
    [switch]$TestCandidate,
    [switch]$RequireSignature
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
    throw 'Release executable is missing. Run scripts/build_release.ps1 from a clean commit first.'
}
$binaryVersion = (Get-Item -LiteralPath $Executable).VersionInfo.ProductVersion
if ($binaryVersion -ne $version) { throw "Executable version $binaryVersion differs from Cargo version $version." }
$buildInfoPath = Join-Path (Split-Path $Executable) 'BUILD_INFO.json'
if (-not (Test-Path -LiteralPath $buildInfoPath -PathType Leaf)) { throw 'Build provenance is missing. Run scripts/build_release.ps1 from the clean release commit.' }
$buildInfo = Get-Content -LiteralPath $buildInfoPath -Raw | ConvertFrom-Json
$binaryHash = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
Push-Location $projectRoot
try {
    $sourceCommit = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Release packaging requires a Git checkout.' }
    $status = git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'Release packaging requires a clean checkout.' }
} finally { Pop-Location }
if ($buildInfo.source_commit -ne $sourceCommit -or $buildInfo.version -ne $version -or $buildInfo.executable_sha256 -ne $binaryHash) {
    throw 'Executable provenance does not match this source commit, version and hash. Rebuild before packaging.'
}
$signature = Get-AuthenticodeSignature -LiteralPath $Executable
$signed = $signature.Status -eq 'Valid'
if ($RequireSignature -or $buildInfo.signing) {
    if (-not $signed -or -not $signature.TimeStamperCertificate -or -not $buildInfo.signing) {
        throw 'Signed release packaging requires a valid timestamped signature and signing provenance.'
    }
    if ($signature.SignerCertificate.Thumbprint -ne $buildInfo.signing.certificate_thumbprint -or
        $signature.TimeStamperCertificate.Thumbprint -ne $buildInfo.signing.timestamp_certificate_thumbprint) {
        throw 'Signature certificates do not match the signed build provenance.'
    }
} elseif ($signature.Status -ne 'NotSigned') {
    throw 'Unexpected signature status; do not package a signature that has not been verified.'
}
$signatureInfo = if ($signed) {
    "Digitally signed by $($signature.SignerCertificate.Subject). Verify SHA256SUMS.txt.`r`nWindows may still show a SmartScreen reputation warning for a new release."
} else {
    'This unsigned build may show an unknown-publisher warning. Verify SHA256SUMS.txt.'
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$staging = Join-Path $OutputDirectory 'Diskvale'
New-Item -ItemType Directory -Path $staging -Force | Out-Null
foreach ($name in @('LICENSE', 'THIRD_PARTY_NOTICES.txt')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination (Join-Path $staging $name) -Force
}
Copy-Item -LiteralPath $Executable -Destination (Join-Path $staging 'Diskvale.exe') -Force
Copy-Item -LiteralPath $buildInfoPath -Destination (Join-Path $staging 'BUILD_INFO.json') -Force
$distributionLabel = if ($TestCandidate) { 'Unpublished test candidate - Early preview' } else { 'Early preview' }
$downloadInfo = if ($TestCandidate) {
    "Unpublished test candidate. Public release awaits a successful live-drive smoke test.`r`nSource: https://github.com/colin-automates/Diskvale/commit/$sourceCommit"
} else {
    "This preview and its downloads:`r`nhttps://github.com/colin-automates/Diskvale/releases/tag/v$version"
}
$quickStart = @"
Diskvale $version - $distributionLabel - Disk space, clearly.
Source commit: $sourceCommit

1. Extract this ZIP into a folder.
2. Open Diskvale.exe and approve the Windows administrator prompt.
3. Choose an NTFS, exFAT or FAT32 drive and click Scan drive.
   For newly connected external drives, choose Refresh drives first.
   Use Rescan to refresh results. Opening the app does not start a scan.

Windows 10/11 x64, a whole local drive and administrator access are required.
No installer, development tools or separate Visual C++ runtime are needed.
$signatureInfo
The NTFS reader is experimental; unsupported metadata returns an explicit error.
exFAT/FAT32 uses a slower read-only metadata scan. Its file totals exclude
directory allocation and filesystem metadata; deletion is disabled for those scans.
Large drives may need substantial memory. Full-volume correctness and performance
parity with other tools have not been established.

Map: explore storage visually. Double-click folders; Backspace goes up.
Files: find the largest files. Types: see totals by extension.
Search example: ext:mp4 >1GB. Ctrl+K focuses search.

Delete actions ask you to confirm the exact target. If recycling is refused,
permanent deletion requires a separate confirmation and cannot be undone.
Deletion uses best-effort identity checks, not an atomic filesystem operation.
Saved, partial and stale results cannot authorize deletion. Rescan after changes.
Snapshots and CSVs contain filenames and paths; redact them before sharing.

$downloadInfo

Report a problem (include version and redacted reproduction/error text):
https://github.com/colin-automates/Diskvale/issues/new?template=bug_report.yml

Full source, documentation, updates and demo:
https://github.com/colin-automates/Diskvale

Code signing policy:
https://github.com/colin-automates/Diskvale/blob/main/CODE_SIGNING.md

Diskvale is MIT licensed. See LICENSE and THIRD_PARTY_NOTICES.txt.
"@
[IO.File]::WriteAllText((Join-Path $staging 'README.txt'), $quickStart + "`r`n", [Text.UTF8Encoding]::new($false))
$zip = Join-Path $OutputDirectory 'Diskvale-windows-x64.zip'
# An explicit list keeps unrelated files from an older staging folder out.
$packageFiles = @('Diskvale.exe', 'README.txt', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'BUILD_INFO.json') | ForEach-Object { Join-Path $staging $_ }
Compress-Archive -LiteralPath $packageFiles -DestinationPath $zip -Force
Copy-Item -LiteralPath (Join-Path $staging 'Diskvale.exe') -Destination (Join-Path $OutputDirectory 'Diskvale.exe') -Force
Copy-Item -LiteralPath (Join-Path $staging 'THIRD_PARTY_NOTICES.txt') -Destination (Join-Path $OutputDirectory 'THIRD_PARTY_NOTICES.txt') -Force
Copy-Item -LiteralPath $buildInfoPath -Destination (Join-Path $OutputDirectory 'BUILD_INFO.json') -Force
$demo = Join-Path $projectRoot 'docs/media/diskvale-demo.mp4'
if (Test-Path -LiteralPath $demo) { Copy-Item -LiteralPath $demo -Destination (Join-Path $OutputDirectory 'Diskvale-Demo.mp4') -Force }
$downloads = @('Diskvale.exe', 'Diskvale-windows-x64.zip', 'THIRD_PARTY_NOTICES.txt', 'BUILD_INFO.json')
if (Test-Path -LiteralPath $demo) { $downloads += 'Diskvale-Demo.mp4' }
$hashes = foreach ($name in $downloads) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $OutputDirectory $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $name"
}
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'SHA256SUMS.txt'), ($hashes -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
Write-Output "Packaged Diskvale $version in $OutputDirectory"
Write-Output $hashes
