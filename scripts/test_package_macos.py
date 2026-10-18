"""Package integrity checks without building or invoking macOS tools."""
import contextlib
import hashlib
import io
from pathlib import Path
import sys
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import package_macos


class BundleIntegrityTests(unittest.TestCase):
    def test_transitive_rpath_is_resolved_at_its_original_library(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);host=root/'host/lib';host.mkdir(parents=True)
            original=host/'libwebp.dylib';original.touch()
            sharp=host/'libsharpyuv.0.dylib';sharp.touch()
            executable=root/'build/photosort'
            load_commands='Load command 3\n          cmd LC_RPATH\n      cmdsize 32\n         path @loader_path/../lib (offset 12)\n'
            with patch.object(package_macos.subprocess,'check_output',return_value=load_commands):
                paths=package_macos.loader_paths(original,executable)
            resolved=package_macos.resolve_dependency('@rpath/libsharpyuv.0.dylib',original,executable,paths)
            self.assertEqual(resolved,sharp.resolve())
            with self.assertRaises(RuntimeError):
                package_macos.resolve_dependency('@rpath/missing.dylib',original,executable,paths)
            with self.assertRaises(RuntimeError):
                package_macos.resolve_dependency('@executable_path/../Frameworks/missing.dylib',original,executable,paths)

    def test_private_dependencies_are_bundled_but_unknown_host_paths_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory)/'vips'
            self.assertTrue(package_macos.dependency_is_external(str(prefix/'lib/libvips.dylib'), prefix))
            self.assertFalse(package_macos.dependency_is_external('/usr/lib/libSystem.B.dylib', prefix))
            self.assertFalse(package_macos.dependency_is_external('@executable_path/../Frameworks/libvips.dylib', prefix))
            with self.assertRaises(RuntimeError):
                package_macos.dependency_is_external(str(Path(directory)/'vips-other/lib/libvips.dylib'), prefix)

    def test_native_dependency_notices_and_build_provenance_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);prefix=root/'vips';notices=prefix/'share/licenses/libvips'
            notices.mkdir(parents=True);(notices/'LICENSE').write_text('upstream license')
            (prefix/'photosort-build.json').write_text('{"version":"8.18.7"}')
            contents=root/'Contents'
            package_macos.copy_dependency_notices(prefix/'lib/libvips.dylib',contents,prefix)
            self.assertEqual((contents/'Resources/DependencyNotices/libvips/LICENSE').read_text(),'upstream license')
            self.assertEqual((contents/'Resources/DependencyNotices/libvips/build.json').read_text(),'{"version":"8.18.7"}')

    def test_manifest_hashes_resources_and_detects_changes_after_signing(self):
        with tempfile.TemporaryDirectory() as directory:
            contents = Path(directory) / "Contents"
            for name in ("MacOS", "Resources", "Frameworks", "_CodeSignature"):
                (contents / name).mkdir(parents=True)
            resource = contents / "Resources" / "model.onnx"
            # Cross multiple chunks; a model-sized read_bytes allocation is forbidden.
            payload = b"model" * 500_000
            resource.write_bytes(payload)
            library = contents / "Frameworks" / "libtest.dylib"
            library.write_bytes(b"signed library")
            executable = contents / "MacOS" / "photosort"
            executable.write_bytes(b"unsigned executable")
            for relative in ("Resources/SHA256SUMS", "Resources/BUNDLE-SHA256SUMS",
                             "_CodeSignature/CodeResources"):
                (contents / relative).write_text("signature metadata")
            with patch.object(Path, "read_bytes", side_effect=AssertionError("unbounded read")):
                before = package_macos.bundle_manifest(contents)
            self.assertEqual(before, (
                hashlib.sha256(b"signed library").hexdigest() + "  Frameworks/libtest.dylib\n"
                + hashlib.sha256(payload).hexdigest() + "  Resources/model.onnx\n"
            ))
            executable.write_bytes(b"final signed executable")
            (contents / "_CodeSignature/CodeResources").write_text("final seal")
            self.assertEqual(package_macos.bundle_manifest(contents), before)
            resource.write_bytes(b"damaged model")
            self.assertNotEqual(package_macos.bundle_manifest(contents), before)

    def test_signing_seals_final_nested_code_and_never_falls_back(self):
        for identity in ('-', 'Apple Development: Explicit Name (TEAMID)'):
            with self.subTest(identity=identity), tempfile.TemporaryDirectory() as directory:
                app = Path(directory)/'PhotoSort.app'
                contents = app/'Contents'
                for name in ('MacOS', 'Resources/runtime', 'Frameworks'):
                    (contents/name).mkdir(parents=True)
                executable = contents/'MacOS/photosort'
                runtime = contents/'Resources/runtime/onnx.dylib'
                dependency = contents/'Frameworks/image.dylib'
                binaries = [executable, runtime, dependency]
                for binary in binaries:
                    binary.write_bytes(b'unsigned')
                def signing(command, **kwargs):
                    if '--sign' in command:
                        self.assertEqual(command[3], identity)
                        path = Path(command[-1])
                        if path == app:
                            # Every nested binary is finalized before resources are sealed.
                            self.assertTrue(all(p.read_bytes() == b'signed' for p in binaries))
                            self.assertEqual((contents/'Resources/BUNDLE-SHA256SUMS').read_text(),
                                             package_macos.bundle_manifest(contents))
                        else:
                            path.write_bytes(b'signed')
                with patch.object(package_macos.subprocess, 'run', side_effect=signing) as run:
                    package_macos.sign_bundle(app, binaries, identity)
                commands = [c.args[0] for c in run.call_args_list if '--sign' in c.args[0]]
                signed = [Path(c[-1]) for c in commands]
                self.assertEqual(set(signed[:-2]), {runtime, dependency})
                self.assertEqual(signed[-2:], [executable, app])
                # Notarization needs timestamps everywhere and the hardened runtime
                # on the executable; ad-hoc signatures support neither.
                named = identity != '-'
                for command, path in zip(commands, signed):
                    self.assertEqual('--timestamp' in command, named)
                    self.assertEqual('runtime' in command, named and path in (executable, app))
                self.assertEqual(run.call_args_list[-1].args[0],
                                 ['codesign', '--verify', '--deep', '--strict', str(app)])
                failure = subprocess.CalledProcessError(1, 'codesign')
                with patch.object(package_macos.subprocess, 'run', side_effect=failure) as run:
                    with self.assertRaises(subprocess.CalledProcessError):
                        package_macos.sign_bundle(app, binaries, identity)
                self.assertEqual(run.call_count, 1)

    def test_minimum_macos_is_the_highest_bundled_requirement(self):
        build = ('Load command 9\n      cmd LC_BUILD_VERSION\n  cmdsize 32\n platform 1\n'
                 '    minos {}\n      sdk 15.0\n   ntools 1\n     tool 3\n  version 1115.7.3\n')
        legacy = 'Load command 8\n      cmd LC_VERSION_MIN_MACOSX\n  cmdsize 16\n  version 10.13\n      sdk 10.14\n'
        cases = (({'a': build.format('11.0'), 'b': legacy}, '13.0'),
                 ({'a': build.format('14.0'), 'b': build.format('15.4'), 'c': legacy}, '15.4'),
                 ({'a': 'a:\n'}, '13.0'))
        for outputs, expected in cases:
            with self.subTest(expected=expected), patch.object(
                    package_macos.subprocess, 'check_output', side_effect=lambda c, **k: outputs[c[-1]]):
                self.assertEqual(package_macos.minimum_macos(outputs), expected)
        with patch.object(package_macos.subprocess, 'check_output',
                          return_value='      cmd LC_BUILD_VERSION\n  cmdsize 32\n'):
            with self.assertRaises(RuntimeError):
                package_macos.minimum_macos(['broken'])

    def test_final_verification_or_resource_change_stops_packaging(self):
        for mode in ('verification-failure', 'resource-change'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                app = Path(directory)/'PhotoSort.app'
                resources = app/'Contents/Resources'
                resources.mkdir(parents=True)
                resource = resources/'model.onnx'
                resource.write_bytes(b'model')
                def signing(command, **kwargs):
                    if '--verify' in command:
                        if mode == 'verification-failure':
                            raise subprocess.CalledProcessError(1, command)
                        resource.write_bytes(b'changed while signing')
                with patch.object(package_macos.subprocess, 'run', side_effect=signing):
                    with self.assertRaises((subprocess.CalledProcessError, RuntimeError)):
                        package_macos.sign_bundle(app, [], '-')

    def test_cli_identity_is_forwarded_and_failed_signing_preserves_old_bundle(self):
        for flag in ([], ['--sign-identity', 'Apple Development: Explicit Name (TEAMID)']):
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root/'resources').mkdir()
                (root/'resources/model.onnx').write_bytes(b'model')
                (root/'LICENSE').write_text('license')
                target = root/'build'
                (target/'release').mkdir(parents=True)
                (target/'release/photosort').write_bytes(b'fixture executable')
                output = root/'PhotoSort.app'
                (output/'Contents').mkdir(parents=True)
                (output/'Contents/Info.plist').write_bytes(b'old bundle sentinel')
                def otool(command, **kwargs):
                    return str(command[-1]) + ':\n'
                failure = subprocess.CalledProcessError(1, 'codesign')
                with patch.object(package_macos, 'ROOT', root), \
                        patch.dict(os.environ, {'CARGO_TARGET_DIR': str(target)}), \
                        patch.object(sys, 'argv', ['package_macos.py', '--output', str(output), *flag]), \
                        patch.object(package_macos.subprocess, 'run'), \
                        patch.object(package_macos.subprocess, 'check_output', side_effect=otool), \
                        patch.object(package_macos, 'sign_bundle', side_effect=failure) as sign:
                    with self.assertRaises(subprocess.CalledProcessError):
                        package_macos.main()
                self.assertEqual(sign.call_args.args[2], flag[-1] if flag else '-')
                self.assertEqual((output/'Contents/Info.plist').read_bytes(), b'old bundle sentinel')
                self.assertEqual((target/'release/photosort').read_bytes(), b'fixture executable')
                self.assertEqual((root/'resources/model.onnx').read_bytes(), b'model')
                self.assertEqual(list(root.glob('photosort-bundle-*')), [])

    def test_empty_signing_identity_is_rejected_before_building(self):
        with patch.object(sys, 'argv', ['package_macos.py', '--sign-identity', '  ']), \
                patch.object(package_macos.subprocess, 'run') as run, \
                contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                package_macos.main()
            self.assertEqual(raised.exception.code, 2)
            run.assert_not_called()

    def test_invalid_output_is_rejected_before_building(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ordinary = root / "ordinary.app"
            ordinary.mkdir()
            (ordinary / "keep.txt").write_text("user file")
            for output in (root / "not-a-bundle", ordinary):
                with self.subTest(output=output), patch.object(sys, "argv", [
                    "package_macos.py", "--output", str(output)
                ]), patch.object(package_macos.subprocess, "run") as run, contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as raised:
                        package_macos.main()
                    self.assertEqual(raised.exception.code, 2)
                    run.assert_not_called()
            self.assertEqual((ordinary / "keep.txt").read_text(), "user file")

    @unittest.skipIf(sys.platform == "win32", "symlink creation requires local Windows privileges")
    def test_output_symlink_is_rejected_before_building(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            link = root / "linked.app"
            link.symlink_to(root / "missing.app", target_is_directory=True)
            with patch.object(sys, "argv", ["package_macos.py", "--output", str(link)]), \
                    patch.object(package_macos.subprocess, "run") as run, \
                    contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    package_macos.main()
                run.assert_not_called()
            self.assertTrue(link.is_symlink())


if __name__ == "__main__":
    unittest.main()
