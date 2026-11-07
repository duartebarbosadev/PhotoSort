"""Windows packaging checks that run on any system, without building or Windows tools."""
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zipfile

import package_windows


def pe_file(imports=(), delayed=(), old_delay_format=False, image_base=0x140000000):
    """A minimal PE32+ image with one section holding import and delay-import tables."""
    section_rva, section_raw, section_size = 0x1000, 0x200, 0x400
    body = bytearray(section_size)
    # Names after the tables, which take at most 0x200 bytes here.
    names, cursor = {}, 0x200
    for name in (*imports, *delayed):
        names[name] = section_rva + cursor
        encoded = name.encode('ascii') + b'\0'
        body[cursor:cursor+len(encoded)] = encoded
        cursor += len(encoded)
    for index, name in enumerate(imports):
        struct.pack_into('<5I', body, 20*index, 0, 0, 0, names[name], 0)
    delay_offset = 0x100
    for index, name in enumerate(delayed):
        attributes, address = (0, names[name] + image_base) if old_delay_format else (1, names[name])
        struct.pack_into('<8I', body, delay_offset + 32*index, attributes, address, 0, 0, 0, 0, 0, 0)
    header = bytearray(section_raw)
    header[:2] = b'MZ'
    struct.pack_into('<I', header, 0x3c, 0x40)
    header[0x40:0x44] = b'PE\0\0'
    coff = 0x44
    optional_size = 112 + 16*8
    struct.pack_into('<HHIIIHH', header, coff, 0x8664, 1, 0, 0, 0, optional_size, 0x22)
    optional = coff + 20
    struct.pack_into('<H', header, optional, 0x20b)
    struct.pack_into('<Q', header, optional+24, image_base)
    struct.pack_into('<I', header, optional+108, 16)
    directories = optional + 112
    if imports:
        struct.pack_into('<II', header, directories + 8, section_rva, 20*(len(imports)+1))
    if delayed:
        struct.pack_into('<II', header, directories + 8*13, section_rva + delay_offset, 32*(len(delayed)+1))
    table = optional + optional_size
    header[table:table+8] = b'.idata\0\0'
    struct.pack_into('<IIII', header, table+8, section_size, section_rva, section_size, section_raw)
    return bytes(header) + bytes(body)


class PeImportTests(unittest.TestCase):
    def test_reads_direct_and_delay_loaded_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'photosort.exe'
            path.write_bytes(pe_file(['KERNEL32.dll', 'VCRUNTIME140.dll'], ['gstreamer-1.0-0.dll', 'glib-2.0-0.dll']))
            self.assertEqual(package_windows.pe_imports(path),
                             ['KERNEL32.dll', 'VCRUNTIME140.dll', 'gstreamer-1.0-0.dll', 'glib-2.0-0.dll'])

    def test_reads_old_style_delay_import_addresses(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'old.dll'
            path.write_bytes(pe_file(delayed=['avcodec-61.dll'], old_delay_format=True, image_base=0x10000000))
            self.assertEqual(package_windows.pe_imports(path), ['avcodec-61.dll'])

    def test_rejects_files_that_are_not_64_bit_pe_images(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'notes.txt'
            path.write_bytes(b'not a program')
            with self.assertRaises(RuntimeError):
                package_windows.pe_imports(path)
            image = bytearray(pe_file(['KERNEL32.dll']))
            struct.pack_into('<H', image, 0x44 + 20, 0x10b)
            path.write_bytes(image)
            with self.assertRaises(RuntimeError):
                package_windows.pe_imports(path)


class BundleLibrariesTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.gstreamer = self.root/'gstreamer/bin'
        self.vc = self.root/'vc'
        for folder, names in ((self.gstreamer, ['gstreamer-1.0-0.dll', 'glib-2.0-0.dll', 'avcodec-61.dll', 'unused.dll']),
                              (self.vc, ['vcruntime140.dll', 'msvcp140.dll'])):
            folder.mkdir(parents=True)
            for name in names:
                (folder/name).touch()
        self.sources = [('gstreamer', self.gstreamer), ('app', self.vc)]
        self.imports = {
            'photosort.exe': ['KERNEL32.dll', 'VCRUNTIME140.dll', 'gstreamer-1.0-0.dll'],
            'gstreamer-1.0-0.dll': ['glib-2.0-0.dll', 'api-ms-win-crt-runtime-l1-1-0.dll'],
            'glib-2.0-0.dll': ['vcruntime140.dll', 'KERNEL32.dll'],
            'gstlibav.dll': ['avcodec-61.dll', 'gstreamer-1.0-0.dll'],
            'avcodec-61.dll': ['msvcp140.dll'],
            'onnxruntime.dll': ['KERNEL32.dll', 'MSVCP140.dll'],
        }

    def tearDown(self):
        self.directory.cleanup()

    def bundle(self, roots):
        return package_windows.bundle_libraries(
            roots, self.sources,
            imports=lambda path: self.imports.get(path.name, []),
            is_system=lambda name: name.lower() == 'kernel32.dll' or name.lower().startswith('api-ms-win-'))

    def test_follows_imports_into_the_allowed_folders_only(self):
        bundled, system = self.bundle([(self.root/'photosort.exe', ('gstreamer', 'app')),
                                       (self.root/'gstlibav.dll', ('gstreamer', 'app')),
                                       (self.root/'onnxruntime.dll', ('app',))])
        self.assertEqual({name: key for name, (_, key) in bundled.items()}, {
            'vcruntime140.dll': 'app', 'msvcp140.dll': 'app', 'gstreamer-1.0-0.dll': 'gstreamer',
            'glib-2.0-0.dll': 'gstreamer', 'avcodec-61.dll': 'gstreamer'})
        self.assertEqual(system, ['api-ms-win-crt-runtime-l1-1-0.dll', 'kernel32.dll'])
        # Copied under the file's own name, not the import table's spelling.
        self.assertEqual(bundled['vcruntime140.dll'][0].name, 'vcruntime140.dll')

    def test_a_library_outside_the_allowed_folders_fails(self):
        self.imports['onnxruntime.dll'].append('glib-2.0-0.dll')
        with self.assertRaisesRegex(RuntimeError, 'onnxruntime.dll needs glib-2.0-0.dll'):
            self.bundle([(self.root/'onnxruntime.dll', ('app',))])

    def test_an_unknown_library_fails(self):
        self.imports['avcodec-61.dll'].append('vulkan-1.dll')
        with self.assertRaisesRegex(RuntimeError, 'avcodec-61.dll needs vulkan-1.dll'):
            self.bundle([(self.root/'gstlibav.dll', ('gstreamer', 'app'))])


class FolderTests(unittest.TestCase):
    def test_manifest_and_zip_cover_every_file_under_one_top_level_folder(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)/'PhotoSort'
            (folder/'resources/models').mkdir(parents=True)
            (folder/'photosort.exe').write_bytes(b'exe')
            (folder/'resources/models/a.onnx').write_bytes(b'model')
            manifest = folder/'resources/BUNDLE-SHA256SUMS'
            manifest.write_text('stale')
            text = package_windows.folder_manifest(folder, manifest)
            self.assertEqual([line.split('  ')[1] for line in text.splitlines()],
                             ['photosort.exe', 'resources/models/a.onnx'])
            archive = Path(directory)/'out/PhotoSort-1.0.0-windows-x64.zip'
            package_windows.write_zip(folder, archive)
            with zipfile.ZipFile(archive) as written:
                self.assertEqual(sorted(written.namelist()), [
                    'PhotoSort/photosort.exe', 'PhotoSort/resources/BUNDLE-SHA256SUMS',
                    'PhotoSort/resources/models/a.onnx'])
            self.assertFalse(archive.with_name(archive.name+'.partial').exists())

    def test_every_plugin_is_named_once(self):
        self.assertEqual(len(package_windows.PLUGINS), len(set(package_windows.PLUGINS)))
        self.assertIn('gstplayback', package_windows.PLUGINS)


@unittest.skipUnless(sys.platform == 'win32', 'reads a real Windows executable')
class RealExecutableTests(unittest.TestCase):
    def test_reads_this_python(self):
        self.assertTrue(any(name.lower() == 'kernel32.dll' or name.lower().startswith('python')
                            for name in package_windows.pe_imports(sys.executable)))


if __name__ == '__main__':
    unittest.main()
