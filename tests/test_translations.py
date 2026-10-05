"""Assert the integration is localised for every locale Home Assistant ships."""

from __future__ import annotations

from pathlib import Path

from custom_components.screensight.scripts.gen_translations import supported_locales

TRANSLATIONS_DIR = (
    Path(__file__).resolve().parent.parent
    / "custom_components"
    / "screensight"
    / "translations"
)


def test_every_locale_has_a_translation_file() -> None:
    """The on-disk locale set matches Home Assistant's supported set."""
    on_disk = {path.stem for path in TRANSLATIONS_DIR.glob("*.json")}
    expected = set(supported_locales())
    assert expected, "Home Assistant shipped no locales?"
    assert on_disk == expected


def test_english_source_exists() -> None:
    """The English source translation is always present."""
    assert (TRANSLATIONS_DIR / "en.json").is_file()
