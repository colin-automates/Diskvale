[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$UnsignedExecutable,
    [Parameter(Mandatory=$true)][string]$SignedExecutable,
    [Parameter(Mandatory=$true)][string]$BuildInfo,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-fA-F]{40}$')][string]$ExpectedCertificateThumbprint,
    [Parameter(Mandatory=$true)][string]$SigningRequestId,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$taskUnsigned = [IO.Path]::GetFullPath($UnsignedExecutable)
$taskSigned = [IO.Path]::GetFullPath($SignedExecutable)
$taskInfo = Get-Content -LiteralPath $BuildInfo -Raw | ConvertFrom-Json
$taskUnsignedHash = (Get-FileHash -LiteralPath $taskUnsigned -Algorithm SHA256).Hash.ToLowerInvariant()
if ($taskUnsignedHash -ne $taskInfo.executable_sha256) { throw 'Unsigned build provenance does not match the signing input.' }
$taskSignature = Get-AuthenticodeSignature -LiteralPath $taskSigned
if ($taskSignature.Status -ne 'Valid' -or -not $taskSignature.TimeStamperCertificate) { throw 'A valid, timestamped Authenticode signature is required.' }
if ($taskSignature.SignerCertificate.Thumbprint -ne $ExpectedCertificateThumbprint) { throw 'Signing certificate does not match the approved thumbprint.' }
$taskCodeSigning = $taskSignature.SignerCertificate.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.37' }
if (-not $taskCodeSigning -or -not ($taskCodeSigning.EnhancedKeyUsages | Where-Object { $_.Value -eq '1.3.6.1.5.5.7.3.3' })) { throw 'Expected a code-signing certificate.' }
foreach ($taskPath in @($taskUnsigned, $taskSigned)) {
    $taskVersion = (Get-Item -LiteralPath $taskPath).VersionInfo
    if ($taskVersion.ProductName -ne 'Diskvale' -or $taskVersion.ProductVersion -ne $taskInfo.version -or $taskVersion.FileVersion -ne $taskInfo.version) { throw 'Signed/unsigned version resources do not match the build provenance.' }
}
python (Join-Path $PSScriptRoot 'check_signed_content.py') $taskUnsigned $taskSigned
if ($LASTEXITCODE -ne 0) { throw 'Signing changed the executable content.' }
$taskInfo.executable_sha256 = (Get-FileHash -LiteralPath $taskSigned -Algorithm SHA256).Hash.ToLowerInvariant()
$taskInfo | Add-Member -NotePropertyName signing -NotePropertyValue ([ordered]@{
    provider = 'SignPath'
    request_id = $SigningRequestId
    unsigned_executable_sha256 = $taskUnsignedHash
    certificate_subject = $taskSignature.SignerCertificate.Subject
    certificate_thumbprint = $taskSignature.SignerCertificate.Thumbprint
    timestamp_certificate_subject = $taskSignature.TimeStamperCertificate.Subject
    timestamp_certificate_thumbprint = $taskSignature.TimeStamperCertificate.Thumbprint
    github_run_id = $env:GITHUB_RUN_ID
})
$taskOutput = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $taskOutput) { throw 'Use a new output directory for signed build provenance.' }
New-Item -ItemType Directory -Path $taskOutput | Out-Null
Copy-Item -LiteralPath $taskSigned -Destination (Join-Path $taskOutput 'Diskvale.exe')
[IO.File]::WriteAllText((Join-Path $taskOutput 'BUILD_INFO.json'), ($taskInfo | ConvertTo-Json -Depth 6) + "`n", [Text.UTF8Encoding]::new($false))
Write-Output "Verified signed executable and provenance in $taskOutput"
