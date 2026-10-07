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
DEPENDENCIES = ["glib-2.0", "gio-2.0", "libjpeg", "libpng", "libtiff-4", "lcms2", "zlib"]


def write_build_record(prefix, provenance):
    """Expose the complete private build recipe to Rust's persistent cache key."""
    recipe = hashlib.sha256(json.dumps(provenance, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    (prefix / "photosort-build.json").write_text(json.dumps({**provenance, "recipe": recipe}, indent=2) + "\n")
    pc = prefix / "lib/pkgconfig/vips.pc"
    lines = [line for line in pc.read_text().splitlines() if not line.startswith("photosort_recipe=")]
    pc.write_text("\n".join(lines) + f"\nphotosort_recipe={recipe}\n")
    return recipe


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, default=ROOT / ".tools/vips")
    parser.add_argument("--archive", type=Path, help="Use a previously downloaded, checksum-verified source archive")
    parser.add_argument("--simd", action=argparse.BooleanOptionalAction, default=True, help="Use Highway SIMD (default); --no-simd builds the scalar comparison")
    args = parser.parse_args()
    prefix = args.prefix.expanduser().absolute()
    if prefix.exists() or prefix.is_symlink():
        parser.error("Choose a new prefix; existing installations are never overwritten")
    dependencies = {
        name: subprocess.check_output(["pkg-config", "--modversion", name], text=True).strip()
        for name in DEPENDENCIES + (["libhwy"] if args.simd else [])
    }
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
        subprocess.run([str(python), "-m", "pip", "install", "meson==1.12.1", "ninja==1.13.2"], check=True)
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
