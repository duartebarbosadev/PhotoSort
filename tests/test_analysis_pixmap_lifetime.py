import gc

from PIL import Image
from PyQt6.QtWidgets import QApplication

from core.image_pipeline import ImagePipeline
from ui.advanced_image_viewer import SynchronizedImageViewer


_app = QApplication.instance() or QApplication([])


def test_pillow_pixmap_survives_delayed_viewer_repaints():
    viewer = SynchronizedImageViewer()
    viewer.resize(800, 600)
    viewer.show()

    for index in range(300):
        image = Image.new("RGB", (1024, 768), (index % 255, 60, 90))
        pixmap = ImagePipeline._qpixmap_from_pil(image)
        del image
        gc.collect()

        viewer.set_images_data(
            [{"path": f"image-{index}.jpg", "pixmap": pixmap, "rating": 0}]
        )
        if index % 3 == 0:
            viewer.clear()
        _app.processEvents()

    viewer.close()
    _app.processEvents()
