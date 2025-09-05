import pyexiv2  # noqa: F401  # Must be first on Windows
import os
import pytest

try:
    from src.core.metadata_processor import MetadataProcessor

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)


def _get_first(md: dict, keys: list[str]):
    for k in keys:
        if k in md and md[k] not in (None, "", "Unknown"):
            return md[k]
    return None


class TestSidebarRelevantFieldsExtraction:
    """Ensure we still extract the fields the sidebar displays, even if pyexiv2 changes."""

    @classmethod
    def setup_class(cls):
        cls.test_folder = os.path.join("tests", "samples")
        cls.sample_images = []
        if os.path.exists(cls.test_folder):
            for fn in os.listdir(cls.test_folder):
                if fn.lower().endswith((".png", ".jpg", ".jpeg", ".arw")):
                    cls.sample_images.append(os.path.join(cls.test_folder, fn))

        if not IMPORTS_AVAILABLE:
            pytest.skip(f"Cannot import MetadataProcessor: {IMPORT_ERROR}")

        if not cls.sample_images:
            pytest.skip(f"No sample images in {cls.test_folder}")

    def test_union_of_samples_provides_sidebar_fields(self):
        # Collect metadata for all samples
        all_md = [
            MetadataProcessor.get_detailed_metadata(p) for p in self.sample_images
        ]
        all_md = [m for m in all_md if isinstance(m, dict) and "error" not in m]
        assert all_md, "No valid metadata extracted from samples"

        # 1) Must always have dimensions and megapixels computable per-image
        for md in all_md:
            w = md.get("pixel_width")
            h = md.get("pixel_height")
            assert isinstance(w, int) and w > 0
            assert isinstance(h, int) and h > 0
            assert (w * h) > 0

        # 2) Across the union of images, we should find each camera/technical field
        # using the same key sets the sidebar expects. Some tags are optional and may
        # not be present in our small sample set (we won't fail on those).
        def any_has(keys: list[str]) -> bool:
            return any(_get_first(md, keys) is not None for md in all_md)

        # Camera model (and/or make)
        assert any_has(
            ["Exif.Image.Model", "Xmp.tiff.Model", "EXIF:Model", "Model"]
        ) or any_has(["Exif.Image.Make", "Xmp.tiff.Make", "EXIF:Make", "Make"]), (
            "No sample provided camera make/model in expected tags"
        )

        # Lens
        assert any_has(
            [
                "Exif.Photo.LensModel",
                "Exif.Photo.LensSpecification",
                "Xmp.aux.Lens",
                "LensModel",
                "EXIF:LensModel",
                "LensInfo",
                "EXIF:LensInfo",
            ]
        ), "No sample provided lens info in expected tags"

        # Focal length
        assert any_has(["Exif.Photo.FocalLength", "FocalLength", "EXIF:FocalLength"]), (
            "No sample provided focal length"
        )

        # Aperture
        assert any_has(
            [
                "Exif.Photo.FNumber",
                "Exif.Photo.ApertureValue",
                "FNumber",
                "EXIF:FNumber",
                "EXIF:ApertureValue",
            ]
        ), "No sample provided aperture"

        # Shutter speed / exposure time
        assert any_has(
            [
                "ExposureTime",
                "Exif.Photo.ExposureTime",
                "Exif.Photo.ShutterSpeedValue",
                "EXIF:ExposureTime",
                "EXIF:ShutterSpeedValue",
            ]
        ), "No sample provided shutter speed/exposure time"

        # ISO
        assert any_has(
            [
                "Exif.Photo.ISOSpeedRatings",
                "ISO",
                "EXIF:ISO",
                "EXIF:ISOSpeedRatings",
            ]
        ), "No sample provided ISO"

        # Flash
        assert any_has(["Exif.Photo.Flash", "Flash", "EXIF:Flash"]), (
            "No sample provided flash"
        )

        # White balance
        assert any_has(
            [
                "Exif.Photo.WhiteBalance",
                "WhiteBalance",
                "EXIF:WhiteBalance",
            ]
        ), "No sample provided white balance"

        # Metering
        assert any_has(
            [
                "Exif.Photo.MeteringMode",
                "MeteringMode",
                "EXIF:MeteringMode",
            ]
        ), "No sample provided metering mode"

        # Exposure mode
        assert any_has(
            [
                "Exif.Photo.ExposureMode",
                "ExposureMode",
                "EXIF:ExposureMode",
            ]
        ), "No sample provided exposure mode"

        # Orientation
        assert any_has(
            [
                "Exif.Image.Orientation",
                "Orientation",
                "EXIF:Orientation",
            ]
        ), "No sample provided orientation"

        # Optional: Exposure compensation (not always set in samples)
        _ = any_has(
            [
                "Exif.Photo.ExposureCompensation",
                "ExposureCompensation",
                "EXIF:ExposureCompensation",
            ]
        )

        # Optional: Scene type (not always set in samples)
        _ = any_has(
            [
                "Exif.Photo.SceneCaptureType",
                "SceneCaptureType",
                "EXIF:SceneCaptureType",
            ]
        )

        # Optional: Software (depends on encoder/workflow)
        _ = any_has(
            [
                "Exif.Image.Software",
                "Software",
                "EXIF:Software",
            ]
        )

    def test_deterministic_per_sample_expectations(self):
        # Map filenames to metadata and assert per-file expectations
        md_by_name: dict[str, dict] = {}

        # Helper key groups mirroring the sidebar
        camera_model_keys = [
            "Exif.Image.Model",
            "Xmp.tiff.Model",
            "EXIF:Model",
            "Model",
        ]
        camera_make_keys = [
            "Exif.Image.Make",
            "Xmp.tiff.Make",
            "EXIF:Make",
            "Make",
        ]
        lens_keys = [
            "Exif.Photo.LensModel",
            "Exif.Photo.LensSpecification",
            "Xmp.aux.Lens",
            "LensModel",
            "EXIF:LensModel",
            "LensInfo",
            "EXIF:LensInfo",
        ]
        focal_keys = ["Exif.Photo.FocalLength", "FocalLength", "EXIF:FocalLength"]
        aperture_keys = [
            "Exif.Photo.FNumber",
            "Exif.Photo.ApertureValue",
            "FNumber",
            "EXIF:FNumber",
            "EXIF:ApertureValue",
        ]
        shutter_keys = [
            "ExposureTime",
            "Exif.Photo.ExposureTime",
            "Exif.Photo.ShutterSpeedValue",
            "EXIF:ExposureTime",
            "EXIF:ShutterSpeedValue",
        ]
        iso_keys = [
            "Exif.Photo.ISOSpeedRatings",
            "ISO",
            "EXIF:ISO",
            "EXIF:ISOSpeedRatings",
        ]
        flash_keys = ["Exif.Photo.Flash", "Flash", "EXIF:Flash"]
        wb_keys = ["Exif.Photo.WhiteBalance", "WhiteBalance", "EXIF:WhiteBalance"]
        metering_keys = [
            "Exif.Photo.MeteringMode",
            "MeteringMode",
            "EXIF:MeteringMode",
        ]
        exp_mode_keys = [
            "Exif.Photo.ExposureMode",
            "ExposureMode",
            "EXIF:ExposureMode",
        ]
        orientation_keys = [
            "Exif.Image.Orientation",
            "Orientation",
            "EXIF:Orientation",
        ]

        # Build metadata map
        for p in self.sample_images:
            md = MetadataProcessor.get_detailed_metadata(p)
            assert isinstance(md, dict) and "error" not in md
            name = os.path.basename(p)
            md_by_name[name] = md

        def has(md: dict, keys: list[str]) -> bool:
            return _get_first(md, keys) is not None

        # Expectations for PNG (no camera EXIF expected)
        png_md = md_by_name.get("GAB02000-min.png")
        if png_md:
            assert png_md.get("mime_type") == "image/png"
            assert (
                isinstance(png_md.get("pixel_width"), int) and png_md["pixel_width"] > 0
            )
            assert (
                isinstance(png_md.get("pixel_height"), int)
                and png_md["pixel_height"] > 0
            )
            # Camera/technical fields should be absent for our PNG sample
            assert not has(png_md, camera_make_keys + camera_model_keys)
            assert not has(png_md, lens_keys)
            assert not has(png_md, focal_keys)
            assert not has(png_md, aperture_keys)
            assert not has(png_md, shutter_keys)
            assert not has(png_md, iso_keys)
            assert not has(png_md, flash_keys)
            assert not has(png_md, wb_keys)
            assert not has(png_md, metering_keys)
            assert not has(png_md, exp_mode_keys)
            assert not has(png_md, orientation_keys)

        # Expectations for JPEG (camera EXIF expected)
        jpg_md = md_by_name.get("jpg_sample.jpg")
        if jpg_md:
            assert jpg_md.get("mime_type") == "image/jpeg"
            assert has(jpg_md, camera_model_keys) or has(jpg_md, camera_make_keys)
            assert has(jpg_md, lens_keys)
            assert has(jpg_md, focal_keys)
            assert has(jpg_md, aperture_keys)
            assert has(jpg_md, shutter_keys)
            assert has(jpg_md, iso_keys)
            # Orientation may be absent in this JPG sample; treat as optional
            _ = has(jpg_md, orientation_keys)

        # Expectations for ARW (RAW EXIF expected)
        arw_md = md_by_name.get("arw_sample.ARW")
        if arw_md:
            mime = str(arw_md.get("mime_type", ""))
            assert mime.startswith("image/")
            assert "arw" in mime.lower() or mime in {"image/tiff", "image/x-raw"}
            assert has(arw_md, camera_model_keys) or has(arw_md, camera_make_keys)
            assert has(arw_md, lens_keys)
            assert has(arw_md, focal_keys)
            assert has(arw_md, aperture_keys)
            assert has(arw_md, shutter_keys)
            assert has(arw_md, iso_keys)
            assert has(arw_md, orientation_keys)
