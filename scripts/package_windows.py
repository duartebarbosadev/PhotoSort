"""Build a portable Windows x64 folder (and optionally its zip). Run with Python 3; the app needs no Python.

The folder holds everything PhotoSort loads, so it runs on a Windows machine with
nothing installed:

    PhotoSort/
      photosort.exe
      vcruntime140.dll, ...       Visual C++ runtime, app-local
      resources/                  models, ONNX Runtime, licenses (models::resource_roots)
      gstreamer/bin/              GStreamer and its libraries (playback::portable)
      gstreamer/lib/gstreamer-1.0/  the plugins PhotoSort's playbin pipeline uses

Only the GStreamer plugins in PLUGINS are copied; every library they, photosort.exe
and ONNX Runtime import is found by reading their PE import tables, so nothing is
copied that nothing loads, and a missing library fails the build instead of the app.
"""
from pathlib import Path
import argparse
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = tomllib.loads((ROOT/'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
DEFAULT_GSTREAMER = Path(r'C:\Program Files\gstreamer\1.0\msvc_x86_64')
TRIPLET = 'x64-windows-static-md'

# Plugins (lib/gstreamer-1.0/<name>.dll) for core::playback::portable: playbin with
# a videoflip ! videoscale ! videoconvert ! appsink video sink, and the formats in
# core::model::MediaKind::Video. Decoding is software only (libav, dav1d, opus,
# vorbis); hardware decoders (d3d11, nvcodec) are left out on purpose.
PLUGINS = (
    # Pipeline
    'gstcoreelements',      # filesrc, queue, multiqueue, typefind, fakesink
    'gsttypefindfunctions',  # recognizes containers
    'gstplayback',          # playbin, decodebin, uridecodebin, playsink
    'gstapp',               # appsink
    'gstvideoconvertscale',  # videoconvert, videoscale
    'gstvideofilter',       # videoflip
    # Audio: decoded with the video, played through Windows' audio stack
    'gstaudioconvert',
    'gstaudioresample',
    'gstvolume',
    'gstautodetect',        # autoaudiosink
    'gstwasapi2',
    'gstwasapi',
    'gstdirectsound',
    # Containers: mp4/mov/m4v, mkv/webm, avi, wmv, mpg/mpeg
    'gstisomp4',
    'gstmatroska',
    'gstavi',
    'gstasf',
    'gstmpegpsdemux',
    # Parsers decodebin places before decoders
    'gstvideoparsersbad',   # h264parse, h265parse, mpegvideoparse, vp9parse, av1parse
    'gstaudioparsers',      # aacparse, mpegaudioparse, ac3parse
    # Decoders
    'gstlibav',             # FFmpeg: H.264, HEVC, VP8/9, MPEG-2/4, WMV, AAC, MP3, ...
    'gstdav1d',             # AV1
    'gstopus',
    'gstvorbis',
)

# Visual C++ runtime libraries Microsoft allows next to an application
# (VC\Redist\MSVC\<version>\x64\Microsoft.VC143.CRT and .OpenMP).
VC_REDIST_FOLDERS = ('Microsoft.VC143.CRT', 'Microsoft.VC143.OpenMP')


def pe_imports(path):
    """Names of the DLLs a PE file imports, directly or delay-loaded."""
    data = Path(path).read_bytes()
    u16 = lambda offset: struct.unpack_from('<H', data, offset)[0]
    u32 = lambda offset: struct.unpack_from('<I', data, offset)[0]
    if data[:2] != b'MZ':
        raise RuntimeError(f'Not a PE file: {path}')
    header = u32(0x3c)
    if data[header:header+4] != b'PE\0\0':
        raise RuntimeError(f'Not a PE file: {path}')
    coff = header + 4
    sections, optional_size = u16(coff+2), u16(coff+16)
    optional = coff + 20
    magic = u16(optional)
    if magic != 0x20b:
        raise RuntimeError(f'Not a 64-bit PE file: {path}')
    image_base = struct.unpack_from('<Q', data, optional+24)[0]
    directories = optional + 112
    directory_count = u32(directories - 4)
    table = optional + optional_size

    def offset(rva):
        for index in range(sections):
            entry = table + 40*index
            size, address, raw_size, raw = u32(entry+8), u32(entry+12), u32(entry+16), u32(entry+20)
            if address <= rva < address + max(size, raw_size):
                return raw + rva - address
        raise RuntimeError(f'Address {rva:#x} is outside every section of {path}')

    def string(rva):
        start = offset(rva)
        return data[start:data.index(b'\0', start)].decode('ascii')

    def directory(index):
        if index >= directory_count:
            return 0
        return u32(directories + 8*index)

    names = []
    rva = directory(1)
    while rva:
        entry = offset(rva)
        if not any(data[entry:entry+20]):
            break
        names.append(string(u32(entry+12)))
        rva += 20
    rva = directory(13)
    while rva:
        entry = offset(rva)
        attributes, name = u32(entry), u32(entry+4)
        if not name:
            break
        # Version 1 descriptors hold RVAs; the old format held virtual addresses.
        names.append(string(name if attributes & 1 else name - image_base))
        rva += 32
    return names


def system_library(name):
    """True for libraries every supported Windows has."""
    lower = name.lower()
    if lower.startswith(('api-ms-win-', 'ext-ms-')):
        return True
    system = Path(os.environ.get('SystemRoot', r'C:\Windows'))/'System32'
    return (system/name).is_file()


def bundle_libraries(roots, sources, imports=pe_imports, is_system=system_library):
    """Every library `roots` need, as {lowercase name: (source path, destination key)}.

    `sources` is a list of (destination key, folder) searched in order; each root is
    (path, allowed destination keys). A library found in an allowed folder is bundled
    and its own imports followed; libraries the system provides are left out. Anything
    else fails, since Windows would not find it on a clean machine either.
    """
    bundled = {}
    system = set()
    # Import tables spell names in any case; keep each file's own name.
    listings = [(key, {path.name.lower(): path for path in Path(folder).iterdir() if path.is_file()})
                for key, folder in sources]
    pending = [(Path(path), tuple(allowed)) for path, allowed in roots]
    while pending:
        binary, allowed = pending.pop()
        for name in imports(binary):
            lower = name.lower()
            if lower in bundled or lower in system:
                continue
            found = next(((files[lower], key) for key, files in listings
                          if key in allowed and lower in files), None)
            if found:
                bundled[lower] = found
                # A library's own imports resolve from the same places as its loader's.
                pending.append((found[0], allowed))
            elif is_system(name):
                system.add(lower)
            else:
                raise RuntimeError(f'{binary.name} needs {name}, which is neither bundled nor part of Windows')
    return bundled, sorted(system)


def find_vc_redist():
    """The newest Visual C++ redistributable folder of the installed build tools."""
    if os.environ.get('VCToolsRedistDir'):
        return Path(os.environ['VCToolsRedistDir'])/'x64'
    vswhere = Path(os.environ.get('ProgramFiles(x86)', r'C:\Program Files (x86)'))/'Microsoft Visual Studio/Installer/vswhere.exe'
    if not vswhere.is_file():
        raise RuntimeError('Cannot find vswhere.exe; pass --vc-redist')
    installation = subprocess.check_output(
        [str(vswhere), '-latest', '-products', '*', '-requires',
         'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'],
        text=True).strip()
    if not installation:
        raise RuntimeError('No Visual Studio installation with the C++ build tools; pass --vc-redist')
    vc = Path(installation)/'VC'
    version = (vc/'Auxiliary/Build/Microsoft.VCRedistVersion.default.txt').read_text().strip()
    return vc/'Redist/MSVC'/version/'x64'


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def folder_manifest(folder, exclude):
    """SHA-256 of every file in the folder, as `sha256sum` writes it."""
    lines = []
    for path in sorted(folder.rglob('*')):
        if path.is_file() and path != exclude:
            lines.append(sha256(path)+'  '+path.relative_to(folder).as_posix())
    return '\n'.join(lines)+'\n'


def write_zip(folder, archive):
    """Zip the folder as its own top-level directory."""
    archive.parent.mkdir(parents=True, exist_ok=True)
    partial = archive.with_name(archive.name+'.partial')
    with zipfile.ZipFile(partial, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as output:
        for path in sorted(folder.rglob('*')):
            if path.is_file():
                output.write(path, (Path(folder.name)/path.relative_to(folder)).as_posix())
    partial.replace(archive)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--output', type=Path, help='Destination folder (default: target/PhotoSort)')
    parser.add_argument('--zip', type=Path, help='Also write this zip of the folder')
    parser.add_argument('--no-build', action='store_true', help='Package the release binary an earlier build made, without running Cargo')
    parser.add_argument('--gstreamer', type=Path, help=f'GStreamer MSVC x86_64 installation (default: GSTREAMER_1_0_ROOT_MSVC_X86_64 or {DEFAULT_GSTREAMER})')
    parser.add_argument('--vc-redist', type=Path, help='The x64 folder of the Visual C++ redistributable (default: from the installed build tools)')
    parser.add_argument('--native-prefix', type=Path, help='Private libvips build linked into the app (default: PHOTOSORT_VIPS_PREFIX or .tools/vips); the build makes it if it is missing')
    parser.add_argument('--vcpkg-root', type=Path, help='vcpkg tree the build used, for license notices (default: VCPKG_ROOT or <target>/vcpkg)')
    args = parser.parse_args()
    if sys.platform != 'win32':
        parser.error('Package Windows builds on Windows')

    target = Path(os.environ.get('CARGO_TARGET_DIR', str(ROOT/'target'))).expanduser()
    if not target.is_absolute():
        target = ROOT/target
    target = target.resolve()
    destination = (args.output or target/'PhotoSort').expanduser().absolute()
    if destination.exists() and not (destination/'photosort.exe').is_file():
        parser.error(f'Refusing to replace {destination}, which is not a PhotoSort folder')
    gstreamer = (args.gstreamer or Path(os.environ.get('GSTREAMER_1_0_ROOT_MSVC_X86_64', DEFAULT_GSTREAMER))).resolve()
    if not (gstreamer/'bin/gstreamer-1.0-0.dll').is_file():
        parser.error(f'No GStreamer installation in {gstreamer}')
    vc_redist = (args.vc_redist or find_vc_redist()).resolve()
    vc_folders = [next(vc_redist.glob(name.replace('143', '14*')), None) for name in VC_REDIST_FOLDERS]
    if not all(vc_folders):
        parser.error(f'No Visual C++ runtime folders in {vc_redist}')
    vcpkg_root = (args.vcpkg_root or Path(os.environ.get('VCPKG_ROOT', str(target/'vcpkg')))).resolve()
    vcpkg_share = vcpkg_root/'installed'/TRIPLET/'share'
    if not vcpkg_share.is_dir():
        parser.error(f'No vcpkg {TRIPLET} installation in {vcpkg_root}')
    # libvips is linked into photosort.exe (scripts/build_vips.py); its notices are bundled.
    native_prefix = (args.native_prefix or Path(os.environ.get('PHOTOSORT_VIPS_PREFIX', str(ROOT/'.tools/vips')))).resolve()
    runtime = ROOT/'resources/runtime'
    if not (runtime/'onnxruntime.dll').is_file():
        parser.error('Put ONNX Runtime (onnxruntime.dll, onnxruntime_providers_shared.dll) in resources/runtime first')

    if args.no_build:
        if not (target/'release/photosort.exe').is_file():
            parser.error('--no-build needs a release build; package once without it first')
    else:
        # The libvips build script uses (and if needed makes) this prefix.
        subprocess.run(['cargo', 'build', '--release', '--locked', '--target-dir', str(target)], cwd=ROOT,
                       env={**os.environ, 'PHOTOSORT_VIPS_PREFIX': str(native_prefix)}, check=True)
    if not (native_prefix/'share/licenses/libvips').is_dir() or not (native_prefix/'photosort-build.json').is_file():
        parser.error(f'No libvips build in {native_prefix}; build without --no-build first')

    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent, prefix='photosort-package-') as staging:
        folder = Path(staging)/destination.name
        resources = folder/'resources'
        plugins = folder/'gstreamer/lib/gstreamer-1.0'
        plugins.mkdir(parents=True)
        (folder/'gstreamer/bin').mkdir()
        exe = folder/'photosort.exe'
        shutil.copy2(target/'release/photosort.exe', exe)
        # Libraries for other systems (and their notices' sums) stay out.
        shutil.copytree(ROOT/'resources', resources,
                        ignore=shutil.ignore_patterns('*.dylib', '*.so', '*.so.*'))
        shutil.copy2(ROOT/'LICENSE', resources/'LICENSE')
        for plugin in PLUGINS:
            source = gstreamer/'lib/gstreamer-1.0'/(plugin+'.dll')
            if not source.is_file():
                raise RuntimeError(f'GStreamer plugin {plugin} is missing from {gstreamer}')
            shutil.copy2(source, plugins/source.name)

        # photosort.exe loads GStreamer from gstreamer/bin (playback::portable sets it
        # as the DLL directory) and the Visual C++ runtime from its own folder. ONNX
        # Runtime is loaded by path before GStreamer, so only the runtime folder serves it.
        sources = [('gstreamer', gstreamer/'bin'), *(('app', path) for path in vc_folders)]
        roots = [(exe, ('gstreamer', 'app')),
                 *((path, ('gstreamer', 'app')) for path in sorted(plugins.glob('*.dll'))),
                 *((path, ('app',)) for path in sorted((resources/'runtime').glob('*.dll')))]
        libraries, system = bundle_libraries(roots, sources)
        for source, key in libraries.values():
            shutil.copy2(source, folder/'gstreamer/bin' if key == 'gstreamer' else folder)
        print('Uses these Windows libraries:', ', '.join(system))

        notices = resources/'DependencyNotices'
        shutil.copytree(gstreamer/'share/licenses', notices/'gstreamer')
        for port in sorted(vcpkg_share.iterdir()):
            if (port/'copyright').is_file():
                (notices/'vcpkg'/port.name).mkdir(parents=True)
                shutil.copy2(port/'copyright', notices/'vcpkg'/port.name/'copyright')
        shutil.copytree(native_prefix/'share/licenses/libvips', notices/'libvips')
        shutil.copy2(native_prefix/'photosort-build.json', notices/'libvips/build.json')
        manifest = resources/'BUNDLE-SHA256SUMS'
        manifest.write_text(folder_manifest(folder, manifest), encoding='utf-8', newline='\n')

        if destination.exists():
            shutil.rmtree(destination)
        shutil.move(str(folder), destination)
    if args.zip:
        write_zip(destination, args.zip.expanduser().absolute())
        print(args.zip)
    print(destination)


if __name__ == '__main__':
    main()
