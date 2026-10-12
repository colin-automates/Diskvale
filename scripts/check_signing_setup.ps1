[CmdletBinding()]
param([switch]$DryRun)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_REPOSITORY -ne 'colin-automates/Diskvale' -or $env:GITHUB_REF -ne 'refs/heads/main') {
    throw 'Signing preparation runs only on the Diskvale main branch.'
}
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskCargo = Get-Content -LiteralPath (Join-Path $taskRoot 'Cargo.toml') -Raw
$taskMatch = [regex]::Match($taskCargo, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$')
if (-not $taskMatch.Success) { throw 'Expected a three-part release version in Cargo.toml.' }
$taskVersion = $taskMatch.Groups[1].Value
if (-not $DryRun) {
    foreach ($taskName in @('SIGNPATH_API_TOKEN', 'SIGNPATH_ORGANIZATION_ID', 'SIGNPATH_PROJECT_SLUG', 'SIGNPATH_SIGNING_POLICY_SLUG', 'SIGNPATH_ARTIFACT_CONFIGURATION_SLUG', 'SIGNPATH_CERTIFICATE_THUMBPRINT')) {
        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($taskName))) {
            throw "Missing $taskName. Complete docs/code-signing-setup.md before requesting a signature."
        }
    }
    if ($env:SIGNPATH_CERTIFICATE_THUMBPRINT -notmatch '^[0-9a-fA-F]{40}$') { throw 'Expected the SHA-1 thumbprint of the approved signing certificate.' }
    # Published version identities remain immutable; signing gets a new version.
    Push-Location $taskRoot
    try {
        $taskExisting = git ls-remote --exit-code --tags origin "refs/tags/v$taskVersion"
        $taskCode = $LASTEXITCODE
        if ($taskCode -eq 0 -or $taskExisting) { throw "v$taskVersion already exists. Choose an unused version before signing." }
        if ($taskCode -ne 2) { throw 'Could not verify that the release version is unused.' }
    } finally { Pop-Location }
}
if ($env:GITHUB_OUTPUT) { "version=$taskVersion" | Out-File -FilePath $env:GITHUB_OUTPUT -Encoding utf8 -Append }
Write-Output "Signing preparation checked for Diskvale $taskVersion; dry run: $DryRun"
