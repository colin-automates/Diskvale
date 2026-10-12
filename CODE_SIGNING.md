# Code signing policy

**Status: setup in progress; SignPath Foundation approval and account configuration are required. Diskvale 0.3.27 is unsigned.** No signed release is advertised until its public EXE passes Windows signature verification.

The intended service, subject to approval, is: **Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).** Foundation certificates identify SignPath Foundation as the publisher. This statement describes the proposed arrangement, not an active certificate or endorsement.

## Responsibility

| Responsibility | Maintainer |
| --- | --- |
| Source changes and review | [colin-automates](https://github.com/colin-automates) |
| Release and signing approval | [colin-automates](https://github.com/colin-automates) |

Contributions from outside the maintainer team require maintainer review before signing. Signing accounts and repository accounts used for releases must have multi-factor authentication enabled. Each production signing request requires the maintainer's manual approval in SignPath after checking the source, CI results and intended version.

## Release process

Signed releases are built from committed source on GitHub-hosted Windows runners. Only Diskvale's own executable is submitted. Its product name and versions must match the artifact configuration and Cargo version. Signing credentials stay in GitHub Actions secrets; certificate private keys remain with the signing service.

After signing, the pipeline checks Windows trust, the approved certificate thumbprint, timestamping and preservation of the original executable content. It records signing provenance, then builds the portable ZIP and checksums from the signed EXE. ZIP and standalone EXEs must match. Publication follows the [release checks](docs/release.md), including fresh public downloads. Released tags and binaries keep their identities; a signed release uses an unused version.

The current signing workflow defaults to a dry run, which produces an **unsigned** input artifact and submits no signing request. See [setup instructions](docs/code-signing-setup.md). Code signing establishes provenance and publisher identity; Windows reputation checks may still warn about a new signed release. See [Microsoft's explanation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

## Privacy

Diskvale scans locally and has no telemetry, automatic uploads or automatic update checks. File names and paths remain in memory unless the user saves a snapshot or exports a CSV to a chosen destination. Users can explicitly select network-backed storage; Windows and the selected storage provider control that transfer. Redact saved scans and exports before sharing them.

Signing submits built application binaries and build-origin metadata, not user drive contents. SignPath handles signing-account and build data under its [privacy policy](https://signpath.io/privacy-policy); GitHub handles repository and CI data under its [privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
