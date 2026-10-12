# Exercise signing gates against the real clean-source unsigned release build.
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$taskExecutable = Join-Path $taskRoot 'target/x86_64-pc-windows-msvc/release/canopy.exe'
$taskInfo = Join-Path (Split-Path $taskExecutable) 'BUILD_INFO.json'
$taskOutput = Join-Path $taskRoot ('artifacts/signing-guard-' + [Guid]::NewGuid().ToString('N'))
if ((Get-AuthenticodeSignature -LiteralPath $taskExecutable).Status -ne 'NotSigned') { throw 'Guard fixture must be the raw unsigned build.' }
$taskRejected = $false
try {
    & (Join-Path $taskRoot 'scripts/package_release.ps1') -Executable $taskExecutable -OutputDirectory $taskOutput -RequireSignature
} catch {
    if ($_.Exception.Message -notlike 'Signed release packaging requires*') { throw }
    $taskRejected = $true
}
if (-not $taskRejected -or (Test-Path -LiteralPath $taskOutput)) { throw 'Unsigned input crossed the signed packaging gate.' }
$taskRejected = $false
try {
    & (Join-Path $taskRoot 'scripts/prepare_signed_release.ps1') -UnsignedExecutable $taskExecutable -SignedExecutable $taskExecutable -BuildInfo $taskInfo -ExpectedCertificateThumbprint ('0' * 40) -SigningRequestId 'unsigned-negative-fixture' -OutputDirectory $taskOutput
} catch {
    if ($_.Exception.Message -notlike 'A valid, timestamped Authenticode signature is required*') { throw }
    $taskRejected = $true
}
if (-not $taskRejected -or (Test-Path -LiteralPath $taskOutput)) { throw 'Unsigned input crossed the signed provenance gate.' }
Write-Output 'Signed packaging and provenance gates rejected the real unsigned executable without producing output.'
