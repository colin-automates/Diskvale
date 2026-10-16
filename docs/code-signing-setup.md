# Set up SignPath Foundation signing

The [code signing policy](../CODE_SIGNING.md) records the proposed arrangement. **No Foundation approval or production certificate is active. Current 0.3.28 downloads are unsigned.** These are instructions for a future approved setup; configuration alone cannot provide a production certificate.

1. Apply through [SignPath Foundation](https://signpath.org/apply.html). Supply the public repository, homepage/download page, functionality, honest adoption evidence and the maintainer's chosen account details. The maintainer must review and agree to the applicable terms and data processing. Keep personal contact details out of this repository. Foundation acceptance is discretionary, especially for a newly released project.
2. After approval, enable MFA for the maintainer's SignPath and GitHub accounts. Configure a SignPath project for `colin-automates/Diskvale`, GitHub.com origin verification and GitHub-hosted Windows runners. Install the SignPath GitHub App only for this repository when required by the approved setup; review its permissions before granting access.
3. Create an artifact configuration, suggested slug `diskvale-windows-exe`, using [.signpath/diskvale.xml](../.signpath/diskvale.xml). GitHub's uploaded artifact is a ZIP containing only `Diskvale.exe`. The `version` parameter restricts its product/file versions; product name is `Diskvale`. Do not add upstream EXEs/DLLs to signing scope.
4. Configure a production signing policy, suggested slug `release-signing`, with the Foundation-provided certificate and **manual approval for every signing request**. Assign `colin-automates` as the release approver and give the CI submitter permission to request signatures, without approval authority. Copy the approved certificate's SHA-1 thumbprint directly from SignPath; it is a public identifier, not a private key.
5. Add `SIGNPATH_API_TOKEN` as a GitHub Actions secret for the CI submitter. Set these repository variables to the exact values from the approved SignPath project:

   | Variable | Value |
   | --- | --- |
   | `SIGNPATH_ORGANIZATION_ID` | Organization ID |
   | `SIGNPATH_PROJECT_SLUG` | Project slug |
   | `SIGNPATH_SIGNING_POLICY_SLUG` | Production policy slug |
   | `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` | Artifact configuration slug |
   | `SIGNPATH_CERTIFICATE_THUMBPRINT` | Approved certificate thumbprint, 40 hexadecimal characters |

6. Run **Prepare signed Windows release** from `main` with **dry_run enabled**. This validates the full build and uploads an unsigned signing-input artifact. It works before credentials are configured and sends no signing request. A dry run is not evidence of a valid production signature.
7. For the first signed release, choose an unused Cargo version and matching lockfile entry, prepare truthful release notes/download links, commit and push `main`. Run the workflow with **dry_run disabled**. Approve the request in SignPath after checking its GitHub-origin build, source/version, tests and artifact metadata. The workflow rejects an existing version tag, missing configuration, invalid/untimestamped signatures, an unexpected certificate and changed executable content.
8. Use all six assets from the successful `Diskvale-signed-windows-x64` artifact together. `BUILD_INFO.json` retains the original source/toolchain and records the signed EXE hash, unsigned hash, signing request, certificates and GitHub run. Follow [release maintenance](release.md) before tagging/uploading/publishing. Verify the final public EXE with `Get-AuthenticodeSignature`, check its publisher and timestamp, compare hashes and recheck ZIP/standalone equality. Mark the policy and download page active only after approval and successful public verification.

The workflow intentionally uploads preparation artifacts and does not publish a GitHub release automatically. Approval timeout/failure does not produce a signed package. A changed or renewed certificate requires maintainer review and an updated expected thumbprint.

## Local validation

```powershell
python -m unittest discover -s scripts/tests -p "test_*.py"
powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1
```

For an approved signed result, `scripts/prepare_signed_release.ps1` checks the original build's hash and certificate/content invariants before recording the signed provenance. `scripts/package_release.ps1 -RequireSignature` rejects unsigned builds. Do not replace this with a self-signed certificate or disable Windows trust checks.

References: [Foundation terms](https://signpath.org/terms.html), [GitHub integration](https://docs.signpath.io/trusted-build-systems/github), [artifact configuration](https://docs.signpath.io/artifact-configuration/reference).
