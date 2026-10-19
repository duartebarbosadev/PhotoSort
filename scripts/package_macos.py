"""Build a relocatable local macOS app. Run with Python 3; app needs no Python."""
from pathlib import Path
import argparse
import os
import plistlib
import shutil
import subprocess
import tempfile
import hashlib
import tomllib

ROOT = Path(__file__).resolve().parents[1]
VERSION = tomllib.loads((ROOT/'Cargo.toml').read_text())['package']['version']
def bundle_manifest(contents):
    """Hash finalized resources with bounded memory; codesign verifies executable code."""
    checksums = []
    executable = contents/'MacOS'/'photosort'
    for path in sorted(contents.rglob('*')):
        # The outer app signature changes the executable and seals this manifest.
        # Including it here would create a circular checksum dependency. The OS
        # signature verifies the executable; this list verifies bundled resources.
        if (not path.is_file() or path == executable
                or path.name in ('SHA256SUMS', 'BUNDLE-SHA256SUMS')
                or '_CodeSignature' in path.parts):
            continue
        digest = hashlib.sha256()
        with path.open('rb') as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(chunk)
        checksums.append(digest.hexdigest()+'  '+str(path.relative_to(contents)))
    return '\n'.join(checksums)+'\n'


def dependency_is_external(source, native_prefix):
    path = Path(source)
    if not path.is_absolute() or source.startswith(('/usr/lib/', '/System/Library/')):
        return False
    if source.startswith(('/opt/homebrew/', '/usr/local/')) or path.resolve().is_relative_to(native_prefix.resolve()):
        return True
    raise RuntimeError(f'Unbundled absolute dependency: {source}')


def copy_dependency_notices(source, contents, native_prefix):
    source = Path(source).resolve()
    destination = contents/'Resources'/'DependencyNotices'
    if source.is_relative_to(native_prefix.resolve()):
        notices = native_prefix/'share/licenses/libvips'
        if not notices.is_dir() or not (native_prefix/'photosort-build.json').is_file():
            raise RuntimeError('Native image dependency is missing license/build provenance')
        shutil.copytree(notices, destination/'libvips', dirs_exist_ok=True)
        shutil.copy2(native_prefix/'photosort-build.json', destination/'libvips/build.json')
        return
    keg = next((parent for parent in source.parents if (parent/'INSTALL_RECEIPT.json').is_file()), None)
    if keg is None:
        raise RuntimeError(f'Cannot locate dependency notices for {source}')
    notices = [p for p in keg.iterdir() if p.is_file() and p.name.upper().startswith(
        ('LICENSE', 'LICENCE', 'COPYING', 'COPYRIGHT', 'LGPL', 'GPL'))]
    if not notices:
        raise RuntimeError(f'No dependency notices found in {keg}')
    target = destination/(keg.parent.name+'-'+keg.name)
    target.mkdir(parents=True, exist_ok=True)
    for notice in notices:
        shutil.copy2(notice, target/notice.name)
    if (keg/'sbom.spdx.json').is_file():
        shutil.copy2(keg/'sbom.spdx.json', target/'sbom.spdx.json')


def library_id(binary):
    lines = subprocess.check_output(['otool', '-D', str(binary)], text=True).splitlines()
    return lines[1].strip() if len(lines) > 1 else None


def loader_paths(binary, executable):
    lines = subprocess.check_output(['otool', '-l', str(binary)], text=True).splitlines()
    paths = []
    for index, line in enumerate(lines):
        if line.strip() == 'cmd LC_RPATH':
            entry = lines[index+2].strip()
            if not entry.startswith('path '):
                raise RuntimeError(f'Invalid LC_RPATH in {binary}')
            value = entry[5:].split(' (offset ', 1)[0]
            value = value.replace('@loader_path', str(binary.parent)).replace('@executable_path', str(executable.parent))
            paths.append(Path(value))
    return paths


def resolve_dependency(source, binary, executable, rpaths):
    if source.startswith('@loader_path/'):
        candidates = [binary.parent/source[len('@loader_path/'):]]
    elif source.startswith('@executable_path/'):
        candidates = [executable.parent/source[len('@executable_path/'):]]
    elif source.startswith('@rpath/'):
        candidates = [path/source[len('@rpath/'):] for path in rpaths]
    elif source.startswith('/'):
        return Path(source)
    else:
        raise RuntimeError(f'Unsupported dependency reference in {binary}: {source}')
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    raise RuntimeError(f'Cannot resolve dependency in {binary}: {source}')


def png_size(path):
    """Pixel width and height from a PNG header, without decoding the image."""
    with path.open('rb') as stream:
        header = stream.read(24)
    if len(header) < 24 or header[:8] != b'\x89PNG\r\n\x1a\n' or header[12:16] != b'IHDR':
        raise RuntimeError(f'Not a PNG image: {path}')
    return int.from_bytes(header[16:20], 'big'), int.from_bytes(header[20:24], 'big')


# Point sizes of a macOS app icon; each is also rendered at twice its size.
ICON_SIZES = (16, 32, 128, 256, 512)


def write_app_icon(source, resources):
    """Render AppIcon.icns from the 1024-pixel artwork with the system's sips and iconutil."""
    if png_size(source) != (1024, 1024):
        raise RuntimeError(f'App icon must be a 1024x1024 PNG: {source}')
    output = resources/'AppIcon.icns'
    with tempfile.TemporaryDirectory(prefix='photosort-icon-') as directory:
        iconset = Path(directory)/'AppIcon.iconset'
        iconset.mkdir()
        for size in ICON_SIZES:
            for scale, suffix in ((1, ''), (2, '@2x')):
                pixels = str(size*scale)
                subprocess.run(['sips', '-z', pixels, pixels, str(source), '--out',
                                str(iconset/f'icon_{size}x{size}{suffix}.png')],
                               check=True, stdout=subprocess.DEVNULL)
        subprocess.run(['iconutil', '--convert', 'icns', '--output', str(output), str(iconset)], check=True)
    if not output.is_file() or output.stat().st_size == 0:
        raise RuntimeError('iconutil did not write AppIcon.icns')
    return output


def sign_bundle(app, binaries, identity):
    """Sign nested code and seal finalized resources with one explicit identity."""
    contents = app/'Contents'
    executable = contents/'MacOS/photosort'
    for binary in sorted(binaries, key=lambda p: p == executable):
        subprocess.run(['codesign', '--force', '--sign', identity, str(binary)], check=True)
    checksums = bundle_manifest(contents)
    (contents/'Resources'/'BUNDLE-SHA256SUMS').write_text(checksums)
    subprocess.run(['codesign', '--force', '--sign', identity, str(app)], check=True)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    if bundle_manifest(contents) != checksums:
        raise RuntimeError('Bundle resources changed while signing')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, help='Destination .app bundle (default: target/PhotoSort.app)')
    parser.add_argument('--native-imaging', action='store_true', help='Include the opt-in libvips backend built by scripts/build_vips.py')
    parser.add_argument('--native-prefix', type=Path, help='Private libvips prefix to bundle with --native-imaging (default: .tools/vips)')
    parser.add_argument('--sign-identity', default='-', help='Existing codesign identity name or SHA-1; default: ad-hoc (-). No notarization is performed.')
    args = parser.parse_args()
    if not args.sign_identity.strip():
        parser.error('--sign-identity must name an existing identity, or - for ad-hoc signing')
    if args.native_prefix and not args.native_imaging:
        parser.error('--native-prefix requires --native-imaging')
    target = Path(os.environ.get('CARGO_TARGET_DIR', str(ROOT/'target'))).expanduser()
    if not target.is_absolute():
        target = ROOT/target
    target = target.resolve()
    destination = (args.output or ROOT/'target'/'PhotoSort.app').expanduser().absolute()
    if destination.suffix != '.app' or destination.is_symlink():
        parser.error('--output must name a non-symlink .app bundle')
    if destination.exists() and not (destination/'Contents'/'Info.plist').is_file():
        parser.error('Refusing to replace a directory that is not an app bundle')
    native_prefix = (args.native_prefix or ROOT/'.tools/vips').expanduser().resolve()
    env = os.environ.copy()
    command = ['cargo', 'build', '--release', '--locked', '--target-dir', str(target)]
    if args.native_imaging:
        if not (native_prefix/'lib/pkgconfig/vips.pc').is_file():
            parser.error('Run python3 scripts/build_vips.py before packaging --native-imaging')
        command += ['--features', 'native-imaging']
        env['PKG_CONFIG_PATH'] = str(native_prefix/'lib/pkgconfig') + os.pathsep + env.get('PKG_CONFIG_PATH', '')
    subprocess.run(command, cwd=ROOT, env=env, check=True)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent, prefix='photosort-bundle-') as staging:
        app = Path(staging) / 'PhotoSort.app'
        contents = app / 'Contents'
        for name in ('MacOS', 'Resources', 'Frameworks'):
            (contents/name).mkdir(parents=True, exist_ok=True)
        exe = contents/'MacOS'/'photosort'
        shutil.copy2(target/'release/photosort', exe)
        shutil.copytree(ROOT/'resources', contents/'Resources', dirs_exist_ok=True)
        shutil.copy2(ROOT/'LICENSE', contents/'Resources/LICENSE')
        icon = write_app_icon(ROOT/'resources/icons/app_icon.png', contents/'Resources')
        plist = dict(CFBundleExecutable='photosort', CFBundleIdentifier='dev.duartebarbosa.photosort.rust',
                     CFBundleName='PhotoSort', CFBundleDisplayName='PhotoSort', CFBundleIconFile=icon.stem,
                     CFBundleVersion='1', CFBundleShortVersionString=VERSION, CFBundlePackageType='APPL',
                     NSHighResolutionCapable=True, LSMinimumSystemVersion='13.0')
        (contents/'Info.plist').write_bytes(plistlib.dumps(plist))
        visited = set()
        library_sources = {}
        executable_source = target/'release/photosort'
        def bundle(binary, original, inherited_rpaths=()):
            if binary in visited:
                return
            visited.add(binary)
            rpaths = [*loader_paths(original, executable_source), *inherited_rpaths]
            own_id = library_id(original)
            lines = subprocess.check_output(['otool', '-L', str(original)], text=True).splitlines()[1:]
            for line in lines:
                source = line.strip().split(' (', 1)[0]
                if source == own_id:
                    continue
                resolved = resolve_dependency(source, original, executable_source, rpaths)
                if dependency_is_external(str(resolved), native_prefix):
                    canonical = resolved.resolve()
                    resolved = canonical
                    previous = library_sources.setdefault(resolved.name, canonical)
                    if previous != canonical:
                        raise RuntimeError(f'Conflicting libraries share a filename: {resolved} and {previous}')
                    copy_dependency_notices(resolved, contents, native_prefix)
                    destination = contents/'Frameworks'/resolved.name
                    if not destination.exists():
                        shutil.copy2(resolved, destination)
                    subprocess.run(['install_name_tool', '-change', source,
                                    '@executable_path/../Frameworks/'+destination.name, str(binary)], check=True)
                    bundle(destination, resolved, rpaths)
            if binary.suffix == '.dylib':
                subprocess.run(['install_name_tool', '-id', '@executable_path/../Frameworks/'+binary.name, str(binary)], check=True)
        bundle(exe, executable_source)
        for binary in contents.glob('Resources/runtime/*.dylib'):
            bundle(binary, ROOT/'resources/runtime'/binary.name, loader_paths(executable_source, executable_source))
        # Resolve every non-system dependency against the finished bundle. Signing
        # alone cannot detect missing @rpath libraries or dependencies on the host.
        for binary in visited:
            own_id = library_id(binary)
            lines = subprocess.check_output(['otool', '-L', str(binary)], text=True).splitlines()[1:]
            for line in lines:
                dependency = line.strip().split(' (', 1)[0]
                if dependency == own_id or dependency.startswith(('/usr/lib/', '/System/Library/')):
                    continue
                resolved = resolve_dependency(dependency, binary, exe, loader_paths(binary, exe))
                if not resolved.is_relative_to(contents.resolve()) or not resolved.is_file():
                    raise RuntimeError(f'Bundle still depends on host or missing file: {dependency}')
        sign_bundle(app, visited, args.sign_identity)
        if destination.exists():
            shutil.rmtree(destination)
        shutil.move(str(app), destination)
    print(destination)


if __name__ == '__main__':
    main()
