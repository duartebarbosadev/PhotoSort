# Desktop release packaging

PhotoSort desktop artifacts are built exclusively from `PhotoSort.spec` by
`.github/workflows/release-build.yml`. Do not duplicate PyInstaller options in
the workflow; add required data, binaries, exclusions, or hidden imports to the
spec and keep `core.packaging_smoke.REQUIRED_PACKAGED_MODULES` aligned.

## Artifacts

- Windows CPU: `PhotoSort-Windows-x64.zip`
- Windows CUDA: `PhotoSort-Windows-x64-CUDA.zip`
- Apple Silicon macOS: `PhotoSort-macOS-AppleSilicon.dmg`

Windows uses PyInstaller one-folder mode because the application contains large
native ML libraries. Extracting the ZIP once avoids the repeated temporary
unpacking cost of a one-file executable.

## Release checks

Every packaged application must pass both checks before it is uploaded:

1. `--packaging-smoke-test` imports all deferred workflows and ML backends.
2. `--smoke-test --smoke-delay-ms 1500` constructs and displays the real UI.

The workflow also enforces startup and compressed-artifact size budgets and
uploads PyInstaller's warning and module-cross-reference reports.

## Optional code-signing secrets

Signing steps are skipped when their credentials are absent. Configure these
GitHub Actions secrets to publish trusted artifacts:

### Windows

- `WINDOWS_CERTIFICATE_PFX`: base64-encoded PFX certificate
- `WINDOWS_CERTIFICATE_PASSWORD`: PFX password

### macOS

- `MACOS_CERTIFICATE_P12`: base64-encoded Developer ID Application certificate
- `MACOS_CERTIFICATE_PASSWORD`: certificate password
- `MACOS_SIGNING_IDENTITY`: full Developer ID Application identity
- `APPLE_ID`: notarization Apple ID
- `APPLE_APP_PASSWORD`: app-specific password
- `APPLE_TEAM_ID`: Apple developer team ID

## Rotation model

The ONNX rotation model is deliberately not embedded in the application. Users
place `orientation_model*.onnx` in the persistent Models Folder opened from the
PhotoSort About dialog. The directory is under the platform's application-data
root and therefore survives upgrades and PyInstaller temporary-directory cleanup.
