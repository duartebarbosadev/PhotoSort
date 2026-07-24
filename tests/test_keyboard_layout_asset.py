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

    assert "Cmd/Ctrl+Shift · Hide left panel" in shared
    assert "Cmd/Ctrl+Alt/Option · Step 5 Cull" in shared
    assert "Skip" not in easy_delete
    assert "Skip" not in fix_rotation
    assert "Skip" not in pick_best
    assert "Shift+Enter Apply" in easy_delete
    assert "R · Reset current · Shift+R · Reset all" in easy_delete
    assert "1 · Toggle left image Keep / Trash" in easy_delete
    assert "2 · Toggle right image Keep / Trash" in easy_delete
    assert "Confirm visible suggestions" in easy_delete
    assert "Trash left image" not in easy_delete
    assert "Trash right image" not in easy_delete
    assert "Q · −90° override" in fix_rotation
    assert "E · +90° override" in fix_rotation
    assert "R · Reset current · Shift+R · Reset all" in fix_rotation
    assert "Previous comparison / cluster" in pick_best
    assert "1 · Toggle image 1 Keep / Trash" in pick_best
    assert "2 · Toggle image 2 Keep / Trash" in pick_best
    assert "R · Reset current · Shift+R · Reset all" in pick_best
    for section in (organize, easy_delete, fix_rotation, pick_best, cull):
        assert "Shift+Enter · Apply" in section
