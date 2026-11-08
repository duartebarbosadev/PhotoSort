# Releasing PhotoSort

This guide is for maintainers. It covers the macOS app bundle, the release workflow, the
converted models and macOS signing. For building and testing, see
[CONTRIBUTING.md](../CONTRIBUTING.md).

## macOS app bundle

```sh
python3 scripts/package_macos.py
```

[scripts/package_macos.py](../scripts/package_macos.py) builds a release binary and writes a
relocatable `target/PhotoSort.app`. The bundle holds `resources/` (with any models and ONNX
Runtime there), the linked native libraries, the private libvips build from `.tools/vips`
and their license notices. The script then verifies the signature and the resource manifest.

| Option | Effect |
| --- | --- |
| `--output /path/to/PhotoSort.app` | Writes the bundle elsewhere. |
| `--native-prefix PATH` | Bundles another libvips prefix. |
| `--sign-identity NAME` | Signs with an existing codesign identity name or SHA-1, with a secure timestamp and the hardened runtime. |

Without `--sign-identity` the bundle is ad-hoc signed and not notarized, so macOS may ask
again for folder access after a rebuild. The bundle's minimum macOS is the highest any of its
binaries needs. Homebrew libraries need the macOS version they were built for.

## Update checks

The app checks the GitHub releases of the repository set with
`PHOTOSORT_RELEASE_REPOSITORY=owner/repository` at build time. Builds without it make no
update requests. Tags must be semantic versions such as `3.0.1` or `v3.0.1`. Drafts,
prereleases and older versions are ignored. The app links to the release page and never
installs an update itself.

## Releases

[.github/workflows/release.yml](../.github/workflows/release.yml) builds the macOS app for
Apple Silicon and a Windows x64 zip, and drafts a GitHub release with both.

1. Check that `main` has what you want to ship and its checks pass.
2. Open **Actions → Release → Run workflow** and enter the version, for example `3.0.1`. Or
   run `gh workflow run release.yml -f version=3.0.1`.
3. Review the draft release, replace the summary placeholder in its notes, and publish it.

The workflow sets the version in `Cargo.toml` and `Cargo.lock`, commits "Release 3.0.1" to
`main` and tags it `v3.0.1`. It refuses an existing tag or a version older than the current
one. Pushing a `v*` tag by hand also starts a release; the tag must match `Cargo.toml`.

Each job downloads ONNX Runtime and the seven models, checks them against
`resources/SHA256SUMS`, packages the app with
`PHOTOSORT_RELEASE_REPOSITORY=duartebarbosadev/PhotoSort`, and runs `photosort
--check-runtime` on the result. The Windows job also runs `examples/verify_media.rs` from the
unpacked zip, with the installed GStreamer moved away.

| Asset | Contents |
| --- | --- |
| `PhotoSort-<version>-macos-arm64.zip` | The app bundle |
| `PhotoSort-<version>-windows-x64.zip` | A `PhotoSort` folder: `photosort.exe`, the Visual C++ and OpenMP DLLs, `resources\` and `gstreamer\` |
| `PhotoSort-<version>-SHA256SUMS.txt` | Checksums of both zips |

[scripts/package_windows.py](../scripts/package_windows.py) copies only the GStreamer plugins
in its `PLUGINS` list. To play a new format, add its plugin there. It copies every library
the executable, plugins and ONNX Runtime import, and fails on one it cannot find.

The release notes come from [.github/release-notes.md](../.github/release-notes.md). The
app's update dialog shows them as plain text, so keep them readable without Markdown.

**Test build.** Leave the version empty, or run `gh workflow run release.yml --ref <branch>`.
The workflow builds and checks everything, including signing when the secrets are set. It
keeps the zips as the `photosort-macos-arm64` and `photosort-windows-x64` artifacts and the
checksums and notes as `release`, for 14 days. It creates no commit, tag or release.

## Converted models

`aesthetic.onnx`, `face_detector.onnx`, `face_landmarks_detector.onnx` and
`face_blendshapes.onnx` have no upstream download (see
[resources/ASSETS.md](../resources/ASSETS.md)). Release builds and the app download them from
this repository's `models-2` prerelease and check them against `resources/SHA256SUMS`.
Create a models prerelease from a folder of files that match those pins:

```sh
gh release create models-2 --repo duartebarbosadev/PhotoSort --target main --prerelease \
  --title "Converted models 2" \
  --notes "Converted ONNX models for release builds; provenance and licenses in resources/ASSETS.md. aesthetic.onnx is AGPL-3.0." \
  aesthetic.onnx face_detector.onnx face_landmarks_detector.onnx face_blendshapes.onnx
```

Keep it a prerelease, since the app reads the latest release. Its tag must not start with
`v`, or it starts the release workflow. To change a converted model, publish the next
prerelease (`models-3`) with all four files. Then update the pins in `resources/SHA256SUMS`,
the URLs and sizes in [src/core/model_assets.rs](../src/core/model_assets.rs) and
`MODELS_RELEASE` in the workflow. Leave older prereleases in place so older tags still
build.

## Signing and notarization

Without secrets, releases keep the ad-hoc signature and are not notarized. macOS then asks
users to approve the first launch, and the release notes explain how. To sign and notarize,
set these repository secrets in **Settings → Secrets and variables → Actions**:

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE` | Base64 of a `.p12` export of the Developer ID Application certificate with its private key (`base64 -i certificate.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | The password chosen for that export |
| `APPLE_ID` | Apple Account email of a member of the developer team |
| `APPLE_TEAM_ID` | The ten-character team ID |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple Account |

The two certificate secrets sign with Developer ID, a secure timestamp and the hardened
runtime. All five also notarize the zip with `notarytool` and staple the ticket to the app.
A partial set of either group stops the workflow. The certificate lives in a temporary
keychain that is deleted at the end of the job.

The hardened runtime only loads code signed by the same team. A Developer ID build uses its
bundled ONNX Runtime, and refuses `ORT_DYLIB_PATH` or a Homebrew copy.
