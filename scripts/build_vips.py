"""Build PhotoSort's minimal native image engine without a global installation.

Requires C/C++ build tools, pkg-config, and development libraries for GLib/GIO,
libjpeg, libpng, libtiff, Little CMS, zlib and Highway (pkg-config name libhwy).
Use --no-simd for a scalar comparison build. Python is a build tool only.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import venv

ROOT = Path(__file__).resolve().parents[1]
VERSION = "8.18.7"
SHA256 = "5baaead3b0bb20ffdb9e9ff09aa9fda08620923df77b63b436654cb5e0b3bf94"
URL = f"https://github.com/libvips/libvips/releases/download/v{VERSION}/vips-{VERSION}.tar.xz"
# Build tools from PyPI, pinned by hash. ninja lists every wheel it publishes; pip takes the platform's.
BUILD_TOOLS = """\
meson==1.12.1 --hash=sha256:930bc7542cbd9f57009e182fd014eba48cf1a0180a7b9006d2d32d8f168d8b02
ninja==1.13.2 \\
    --hash=sha256:fd82e26c0706ad4ab88e5fdd26f3fab0a987a90f810160f6c322e752c6af298b \\
    --hash=sha256:d775a5e43e9088f507a6250d57fcf5678eb31268c545feb5064ffeee33735622 \\
    --hash=sha256:81d95081c0ad7c95f67bf682220361ed2a32659d7861b4766b03433c58f22516 \\
    --hash=sha256:227cbc3ae3e5e429692388103cae8c09451df086cd2d342dae0795af0d162547 \\
    --hash=sha256:1684c60d031c54c1d049541b64243c0c567dca5463dbd77682a8901780af293d \\
    --hash=sha256:65a24341b5ac09fcadcc37082660be40a94174e51a937fabf6e2cae26225fa2c \\
    --hash=sha256:aa3d2ae5706a2c4d1e93edc951d1c6cbb45107413c404f8fde1741239efbc9a0 \\
    --hash=sha256:919572cbc3f233261ecd41fe1f3efc9d44aa02464a4588867e06a8b4f6f416ea \\
    --hash=sha256:0e083700470c02ca154a855ae6d692d03564f5064cae52e113896f9ccc078418 \\
    --hash=sha256:59d71c3e15b6b6f3d903eb0c27285544e0747ca59925ada7037bb1af781ad4b3 \\
    --hash=sha256:f90f84affc441e219f15fe52532806c1c9dbd22fb66c3addddce88a3deaabab7 \\
    --hash=sha256:b2f687437fac460b27b7eadc99039b1163016fb4ba7276e2782a192d9f24ee0e \\
    --hash=sha256:09de9ab04f7352f51570c73fd4913acb1e6c24be0a72cd8b80243d4d3ed04925 \\
    --hash=sha256:6a87bf42b123abe2f37737300185f0a303a891899da85d73a3613ee80547e578 \\
    --hash=sha256:915bd482c4be41c75120fd67a22e0bb3f0fbb3bbc5f95b89787deadd59e27ef2 \\
    --hash=sha256:792cadbb9decfd1f776d4d0a6930feb46d08302eb57c176bcf26b09de5748e9f \\
    --hash=sha256:1293f4078278b70d0ee4b6cc8f3a9e030656c9b2f59909970343c4fe76070118 \\
    --hash=sha256:1db9852e528efa7702f5123969f86678663e46d57ff28ab13f5fd84d64a85fb1
"""
DEPENDENCIES = ["glib-2.0", "gio-2.0", "libjpeg", "libpng", "libtiff-4", "lcms2", "zlib"]


def write_build_record(prefix, provenance):
    """Expose the complete private build recipe to Rust's persistent cache key."""
    recipe = hashlib.sha256(json.dumps(provenance, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    (prefix / "photosort-build.json").write_text(json.dumps({**provenance, "recipe": recipe}, indent=2) + "\n")
    pc = prefix / "lib/pkgconfig/vips.pc"
    lines = [line for line in pc.read_text().splitlines() if not line.startswith("photosort_recipe=")]
    pc.write_text("\n".join(lines) + f"\nphotosort_recipe={recipe}\n")
    return recipe


def cache_key(dependencies):
    """CI reuses a built prefix only while this script and the libraries it builds against match."""
    inputs = Path(__file__).read_bytes() + json.dumps(dependencies, sort_keys=True).encode()
    return hashlib.sha256(inputs).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, default=ROOT / ".tools/vips")
    parser.add_argument("--archive", type=Path, help="Use a previously downloaded, checksum-verified source archive")
    parser.add_argument("--simd", action=argparse.BooleanOptionalAction, default=True, help="Use Highway SIMD (default); --no-simd builds the scalar comparison")
    parser.add_argument("--cache-key", action="store_true", help="Print a key for caching the built prefix and exit")
    args = parser.parse_args()
    dependencies = {
        name: subprocess.check_output(["pkg-config", "--modversion", name], text=True).strip()
        for name in DEPENDENCIES + (["libhwy"] if args.simd else [])
    }
    if args.cache_key:
        print(cache_key(dependencies))
        return
    prefix = args.prefix.expanduser().absolute()
    if prefix.exists() or prefix.is_symlink():
        parser.error("Choose a new prefix; existing installations are never overwritten")
    with tempfile.TemporaryDirectory(prefix="photosort-vips-build-") as directory:
        work = Path(directory)
        archive = args.archive.resolve() if args.archive else work / "vips.tar.xz"
        if not args.archive:
            subprocess.run(["curl", "--fail", "--location", "--retry", "2", URL, "--output", str(archive)], check=True)
        with archive.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if digest != SHA256:
            raise RuntimeError(f"libvips source checksum mismatch: {digest}")
        source = work / "source"
        with tarfile.open(archive) as tar:
            tar.extractall(source, filter="data")
        source = source / f"vips-{VERSION}"
        if not (source / "meson.build").is_file():
            raise RuntimeError("Expected libvips source directory is missing")
        tools = work / "tools"
        venv.EnvBuilder(with_pip=True).create(tools)
        binaries = tools / ("Scripts" if os.name == "nt" else "bin")
        python = binaries / ("python.exe" if os.name == "nt" else "python")
        requirements = work / "build-tools.txt"
        requirements.write_text(BUILD_TOOLS)
        subprocess.run([str(python), "-m", "pip", "install", "--require-hashes", "--only-binary=:all:",
                        "-r", str(requirements)], check=True)
        env = os.environ.copy()
        env["PATH"] = str(binaries) + os.pathsep + env.get("PATH", "")
        build = work / "build"
        # Keep the private runtime/pkg-config layout identical on multiarch Linux and macOS.
        options = ["--buildtype=release", "--libdir=lib", "--auto-features=disabled",
                   "-Djpeg=enabled", "-Dpng=enabled", "-Dtiff=enabled", "-Dlcms=enabled", "-Dzlib=enabled",
                   "-Ddeprecated=false", "-Dexamples=false", "-Dcplusplus=false", "-Dmodules=disabled",
                   "-Dnsgif=true", "-Dppm=false", "-Danalyze=false", "-Dradiance=false"]
        if args.simd:
            options.append("-Dhighway=enabled")
        subprocess.run(["meson", "setup", str(build), str(source), "--prefix=" + str(prefix), *options], env=env, check=True)
        subprocess.run(["meson", "compile", "-C", str(build), "-j", "4"], env=env, check=True)
        subprocess.run(["meson", "install", "-C", str(build)], env=env, check=True)
        # Keep the native library's license and exact build provenance with it.
        license_dir = prefix / "share/licenses/libvips"
        license_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / "LICENSE", license_dir / "LICENSE")
        shutil.copy2(source / "libvips/foreign/libnsgif/COPYING", license_dir / "libnsgif-COPYING")
        write_build_record(prefix, {
            "version": VERSION, "source": URL, "sha256": SHA256,
            "dependencies": dependencies, "options": options,
        })
    print(f"Native engine installed in {prefix}")
    print(f"Set PKG_CONFIG_PATH to {prefix / 'lib/pkgconfig'} when building the Rust adapter.")


if __name__ == "__main__":
    main()
