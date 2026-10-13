# macOS development signing and folder access

The packager defaults to an ad-hoc signature (`--sign-identity -`). This is useful
for local builds, but its code identity changes when the executable changes.
macOS may therefore ask again for access to Documents, Desktop or Downloads after
a rebuild. A pending system prompt can leave PhotoSort showing “Discovering
photos” while macOS holds the worker's directory-open call. Bring PhotoSort and
its system access prompt into view and choose whether to allow the requested
folder access. A wait alone does not prove that a permission prompt is the cause.

A Documents-folder permission incident was captured with matching OS evidence during
development. Apple describes code identity and privacy-choice persistence in
[TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).

## Use an existing signing identity

Pass the exact name or SHA-1 of your existing development signing identity:

```sh
python3 scripts/package_macos.py \
  --native-imaging --native-prefix .tools/vips-simd \
  --sign-identity 'Apple Development: Your Name (TEAMID)' \
  --output '/path/to/PhotoSort Rust.app'
```

Replace the example identity and output path with your own. An Apple Development
identity is suitable for development; consistent certificate-backed signing gives
macOS a stable designated requirement across compatible builds. Keep the bundle
identifier and signing identity consistent. Changing signing class or team can
change that requirement and require a new permission decision. Existing ad-hoc
permission grants do not automatically carry over to the first certificate-signed
build. Keychain may ask you to authorize signing with the selected private key.

The packager passes the identity directly to Apple's `codesign` for bundled
libraries, the executable and the outer app. It verifies the complete signature
and resource manifest before replacing the output. A signing failure preserves
the previous app and removes temporary staging; it never silently retries with
ad-hoc signing. Source models, libraries and executables are copied before signing.
No certificates, permissions or custom designated requirements are created.

## Distribution remains separate

This option does not perform notarization or establish distribution readiness.
Developer ID distribution still requires the appropriate identity, hardened runtime
compatibility testing and Apple's notarization/Gatekeeper checks. Do not treat an
ad-hoc strict-signature pass, or an untested certificate selection, as evidence that
the app is ready for public release. Certificate-backed rebuild/permission
persistence has not yet been tested in this workspace.
