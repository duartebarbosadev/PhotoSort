from src.ui.helpers.deletion_utils import build_item_text


def test_build_item_text_unmarked_unblurred():
    assert build_item_text('photo.jpg', False, None) == 'photo.jpg'


def test_build_item_text_marked_only():
    assert build_item_text('photo.jpg', True, None) == 'photo.jpg (DELETED)'


def test_build_item_text_blurred_only():
    assert build_item_text('photo.jpg', False, True) == 'photo.jpg (Blurred)'


def test_build_item_text_marked_and_blurred():
    assert build_item_text('photo.jpg', True, True) == 'photo.jpg (DELETED) (Blurred)'
