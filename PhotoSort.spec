# -*- mode: python ; coding: utf-8 -*-
"""Single cross-platform PyInstaller definition used by release CI."""

import subprocess
import sys
from pathlib import Path

from PyInstaller.utils.hooks import (
    collect_data_files,
    collect_dynamic_libs,
    collect_submodules,
    copy_metadata,
)


PROJECT_ROOT = Path(SPECPATH).resolve()
IS_MACOS = sys.platform == "darwin"


def _homebrew_runtime_libraries():
    if not IS_MACOS:
        return []
    patterns_by_formula = {
        "brotli": ("libbrotlicommon*.dylib", "libbrotlidec*.dylib", "libbrotlienc*.dylib"),
        "inih": ("libinih*.dylib", "libINIReader*.dylib"),
        "gettext": ("libintl*.dylib",),
    }
    binaries = []
    seen = set()
    for formula, patterns in patterns_by_formula.items():
        prefix = Path(
            subprocess.check_output(
                ["brew", "--prefix", formula], text=True
            ).strip()
        )
        for pattern in patterns:
            for library in sorted((prefix / "lib").glob(pattern)):
                resolved = str(library.resolve())
                if resolved not in seen:
                    binaries.append((resolved, "."))
                    seen.add(resolved)
    return binaries


datas = [
    (str(PROJECT_ROOT / "src" / "ui" / "dark_theme.qss"), "."),
    (str(PROJECT_ROOT / "assets" / "app_icon.ico"), "."),
    (str(PROJECT_ROOT / "assets" / "app_icon.png"), "."),
]
datas += collect_data_files(
    "mediapipe",
    includes=[
        "modules/face_detection/**/*",
        "modules/face_landmark/**/*",
        "modules/iris_landmark/**/*",
    ],
)
datas += copy_metadata("pyexiv2")

binaries = collect_dynamic_libs("pyexiv2")
binaries += _homebrew_runtime_libraries()

# AutoModel loads architecture modules by string at runtime. Keep only the
# architectures PhotoSort supports instead of freezing the whole model zoo.
kept_transformer_model_prefixes = (
    "transformers.models.auto",
    "transformers.models.beit",
    "transformers.models.bit",
    "transformers.models.dinov2",
)


def _is_kept_transformer_model(module):
    return module == "transformers.models" or any(
        module == prefix or module.startswith(prefix + ".")
        for prefix in kept_transformer_model_prefixes
    )


excluded_transformer_models = [
    module
    for module in collect_submodules("transformers.models", on_error="ignore")
    if not _is_kept_transformer_model(module)
]

hiddenimports = [
    "core.build_info",
    "core.packaging_smoke",
    "mediapipe.python.solutions.face_mesh",
    "transformers.models.auto.image_processing_auto",
    "transformers.models.auto.modeling_auto",
    "transformers.models.beit.image_processing_beit",
    "transformers.models.beit.modeling_beit",
    "transformers.models.bit.image_processing_bit",
    "transformers.models.dinov2.modeling_dinov2",
    "ui.easy_delete_step_widget",
    "ui.fix_rotation_step_widget",
    "ui.metadata_sidebar",
    "ui.pick_best_step_widget",
    "workers.best_shot_worker",
    "workers.easy_delete_worker",
    "workers.grouping_worker",
    "workers.pick_best_worker",
    "workers.rotation_detection_step_worker",
]

a = Analysis(
    [str(PROJECT_ROOT / "src" / "main.py")],
    pathex=[str(PROJECT_ROOT / "src"), str(PROJECT_ROOT)],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[str(PROJECT_ROOT / "runtime_hook.py")],
    excludes=[
        "PySide2",
        "PySide6",
        "flax",
        "jax",
        "pyiqa",
        "sentence_transformers",
        "tensorflow",
        "tf_keras",
        *excluded_transformer_models,
    ],
    noarchive=False,
    optimize=1,
)
pyz = PYZ(a.pure)

if IS_MACOS:
    generated_macos_icon = PROJECT_ROOT / "assets" / "photosort.icns"
    icon_path = (
        generated_macos_icon
        if generated_macos_icon.exists()
        else PROJECT_ROOT / "assets" / "app_icon.png"
    )
else:
    icon_path = PROJECT_ROOT / "assets" / "app_icon.ico"
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="PhotoSort",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=str(icon_path),
)
coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name="PhotoSort",
)

if IS_MACOS:
    app = BUNDLE(
        coll,
        name="PhotoSort.app",
        icon=str(icon_path),
        bundle_identifier="dev.duartebarbosa.photosort",
        info_plist={
            "CFBundleDisplayName": "PhotoSort",
            "NSHighResolutionCapable": True,
        },
    )
