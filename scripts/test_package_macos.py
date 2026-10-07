"""Package integrity checks without building or invoking macOS tools."""
import contextlib
import hashlib
import io
from pathlib import Path
import sys
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
