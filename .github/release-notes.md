<!-- Release notes template, filled in by .github/workflows/release.yml, which removes
these comments. PhotoSort's update dialog shows the notes as plain text, so keep them
readable without Markdown rendering. -->
PhotoSort ${version} for Apple Silicon Macs.

Summarize the changes here before publishing.

## Download

**${asset}** needs macOS ${minimum_macos} or later on an Apple Silicon Mac. Unzip it and
move **PhotoSort.app** to Applications. The app includes ONNX Runtime ${onnxruntime} and
all AI models, so nothing is downloaded on first use.

<!-- notarized -->
The app is signed with a Developer ID and notarized by Apple.
<!-- /notarized -->
<!-- not-notarized -->
This build is not notarized by Apple, so macOS blocks it the first time you open it. Open
it once, then choose **System Settings → Privacy & Security → Open Anyway**. Alternatively,
remove the download quarantine in Terminal:

```sh
xattr -dr com.apple.quarantine /Applications/PhotoSort.app
```
<!-- /not-notarized -->

SHA-256 (also in **${checksums}**):

```text
${sha256}  ${asset}
```

## Coming from PhotoSort 2.x

PhotoSort 3 is a native rewrite; the Python app ends at
[v2.1.0](https://github.com/${repository}/releases/tag/v2.1.0).

- PhotoSort 2.x announces this release on every platform, but this version is published
  for Apple Silicon Macs only. On Windows, Linux and Intel Macs, keep using 2.1.0.
- PhotoSort 2.x's update prompt opens this page and does not install anything. Download
  the `.zip` above.

## Licenses

PhotoSort is licensed under Apache-2.0. The app bundles third-party components under their
own terms, listed in `PhotoSort.app/Contents/Resources/licenses/` and `DependencyNotices/`:
Exiv2 (GPL-2.0-or-later), LibRaw, libvips and ONNX Runtime among them. The aesthetic model
`aesthetic.onnx` is distributed under AGPL-3.0 per its model card. Its corresponding source
is the pinned upstream revision with
[`scripts/export_aesthetic.py`](https://github.com/${repository}/blob/v${version}/scripts/export_aesthetic.py)
and [`examples/fold_model.rs`](https://github.com/${repository}/blob/v${version}/examples/fold_model.rs),
as described in [resources/ASSETS.md](https://github.com/${repository}/blob/v${version}/resources/ASSETS.md).
