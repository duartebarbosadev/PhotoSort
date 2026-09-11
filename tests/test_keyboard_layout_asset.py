from pathlib import Path


LAYOUT_PATH = Path(__file__).parents[1] / "assets" / "keyboard-layout.html"


def _layout_source() -> str:
    return LAYOUT_PATH.read_text(encoding="utf-8")


def _section(source: str, heading: str, next_heading: str | None = None) -> str:
    section = source.split(f">{heading}</h3>", 1)[1]
    if next_heading is not None:
        section = section.split(f">{next_heading}</h3>", 1)[0]
    return section


def test_keyboard_layout_stays_editable_in_map_maker():
    source = _layout_source()

    assert "https://archie-adams.github.io/keyboard-shortcut-map-maker/" in source
    assert '<ol id="KeyboardTable">' in source
    assert source.count('class="keyboard large"') == 6
    for heading in (
        "Shared controls — available in every workflow step",
        "Step 1 — Organize",
        "Step 2 — Easy Delete",
        "Step 3 — Fix Rotation",
        "Step 4 — Pick Best",
        "Step 5 — Cull",
    ):
        assert f">{heading}</h3>" in source


def test_keyboard_layout_documents_current_workflow_shortcuts():
    source = _layout_source()
    shared = _section(
        source,
        "Shared controls — available in every workflow step",
        "Step 1 — Organize",
    )
    organize = _section(source, "Step 1 — Organize", "Step 2 — Easy Delete")
    easy_delete = _section(source, "Step 2 — Easy Delete", "Step 3 — Fix Rotation")
    fix_rotation = _section(source, "Step 3 — Fix Rotation", "Step 4 — Pick Best")
    pick_best = _section(source, "Step 4 — Pick Best", "Step 5 — Cull")
    cull = _section(source, "Step 5 — Cull")

    assert "Cmd/Ctrl+Shift · Toggle left panel" in shared
    assert "Cmd/Ctrl+Alt/Option · Step 5 Cull" in shared
    assert "Skip" not in easy_delete
    assert "Skip" not in fix_rotation
    assert "Skip" not in pick_best
    assert "Shift+Enter Apply" in easy_delete
    assert "R · Reset current · Shift+R · Reset all" in easy_delete
    assert "1 · Toggle left image Keep / Trash" in easy_delete
    assert "2 · Toggle right image Keep / Trash" in easy_delete
    assert "Confirm all filtered suggestions" in easy_delete
    assert "Trash left image" not in easy_delete
    assert "Trash right image" not in easy_delete
    assert "Q · −90° override" in fix_rotation
    assert "E · +90° override" in fix_rotation
    assert "R · Reset current · Shift+R · Reset all" in fix_rotation
    assert "Previous comparison / cluster" in pick_best
    assert "1 · Toggle image 1 Keep / Trash" in pick_best
    assert "2 · Toggle image 2 Keep / Trash" in pick_best
    assert "R · Reset current · Shift+R · Reset all" in pick_best
    for section in (organize, easy_delete, fix_rotation, pick_best):
        assert "Shift+Enter · Apply" in section
    assert "Shift+Enter · Commit" in cull
    assert "Shift commit" not in organize
    assert "Shift commit" not in cull
    assert "Blur · Cmd/Ctrl best shots" not in cull
    assert 'H<span class="userText"></span>' in cull
    assert 'J<span class="userText"></span>' in cull
    assert 'K<span class="userText"></span>' in cull
    assert 'L<span class="userText"></span>' in cull
    assert cull.count("Cmd/Ctrl includes marked") == 4
    assert "Play / pause selected video" in cull
    assert "Actual size" in cull
    assert "Alt/Option list view" in cull
    assert "Alt/Option icon view" in cull
    assert "Alt/Option grid view" in cull


def test_keyboard_map_arrow_identifiers_match_their_visible_direction():
    import re

    source = _layout_source()
    for code, symbol in (("ArrowUp", "△"), ("ArrowDown", "▽")):
        labels = re.findall(rf'<div data-key="{code}"[^>]*>([^<]+)', source)
        assert labels == [symbol] * 6
    assert 'data-key="Numpad3"' not in source


def test_keyboard_map_scopes_find_to_cull_and_documents_alternate_apply():
    source = _layout_source()
    shared = _section(
        source,
        "Shared controls — available in every workflow step",
        "Step 1 — Organize",
    )
    organize = _section(source, "Step 1 — Organize", "Step 2 — Easy Delete")
    assert "Find" not in shared
    assert "About" in shared
    assert "Help" not in shared
    assert "Cmd/Ctrl+Enter also" in organize
    assert "Cmd/Ctrl Find" in _section(source, "Step 5 — Cull")
    for heading, following in (
        ("Step 2 — Easy Delete", "Step 3 — Fix Rotation"),
        ("Step 3 — Fix Rotation", "Step 4 — Pick Best"),
    ):
        assert "Confirm / cancel" in _section(source, heading, following)


def test_keyboard_guide_links_six_separate_pngs():
    import re
    import struct

    guide_path = LAYOUT_PATH.parents[1] / "docs" / "keyboard-shortcuts.md"
    guide = guide_path.read_text(encoding="utf-8")
    images = re.findall(r"!\[[^\]]*\]\(([^)]+)\)", guide)
    assert len(images) == 6
    assert len(set(images)) == 6
    for target in images:
        data = (guide_path.parent / target).read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n"
        width, height = struct.unpack(">II", data[16:24])
        assert width > height  # One keyboard, not the old tall combined image.
    readme = (LAYOUT_PATH.parents[1] / "README.md").read_text(encoding="utf-8")
    assert "(docs/keyboard-shortcuts.md)" in readme
