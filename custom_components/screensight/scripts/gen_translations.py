"""Regenerate the integration's translations for every Home Assistant locale.

Home Assistant ships dozens of locales and its frontend falls back to English
when a string is missing, but HACS-quality integrations still provide one file
per supported locale so translators can pick them up. This script starts from
``translations/en.json`` and writes a file for every locale present under the
installed ``homeassistant/components/*/translations/`` tree.

Run it with the project environment::

    uv run python -m custom_components.screensight.scripts.gen_translations

A handful of high-traffic strings carry hand-written translations below; every
other locale legitimately falls back to the English source string, since CI has
no machine-translation service available.
"""

from __future__ import annotations

import copy
import importlib.util
import json
import sys
from pathlib import Path

TRANSLATIONS_DIR = Path(__file__).resolve().parent.parent / "translations"
SOURCE_FILE = TRANSLATIONS_DIR / "en.json"

# Dotted ``key.path`` -> translated string, applied on top of the English base.
# Intentionally partial: these are the strings a user sees before the device is
# paired, which is exactly where a missing translation hurts most.
OVERRIDES: dict[str, dict[str, str]] = {
    "fr": {
        "config.step.pair.title": "Associer un écran Screensight",
        "config.step.pair.data.code": "Code d'association",
        "config.step.pair.data.name": "Nom de Home Assistant",
        "config.progress.pair_confirm": "En attente de votre confirmation sur l'écran {device_name}…",
        "entity.text.display_text.name": "Texte affiché",
    },
    "de": {
        "config.step.pair.title": "Screensight-Display koppeln",
        "config.step.pair.data.code": "Kopplungscode",
        "config.step.pair.data.name": "Name von Home Assistant",
        "config.progress.pair_confirm": "Warte auf deine Bestätigung am Display {device_name}…",
        "entity.text.display_text.name": "Anzeigetext",
    },
    "es": {
        "config.step.pair.title": "Emparejar una pantalla Screensight",
        "config.step.pair.data.code": "Código de emparejamiento",
        "config.step.pair.data.name": "Nombre de Home Assistant",
        "config.progress.pair_confirm": "Esperando tu confirmación en la pantalla {device_name}…",
        "entity.text.display_text.name": "Texto mostrado",
    },
    "it": {
        "config.step.pair.title": "Associa un display Screensight",
        "config.step.pair.data.code": "Codice di associazione",
        "config.step.pair.data.name": "Nome di Home Assistant",
        "config.progress.pair_confirm": "In attesa della conferma sul display {device_name}…",
        "entity.text.display_text.name": "Testo visualizzato",
    },
    "pt": {
        "config.step.pair.title": "Emparelhar um ecrã Screensight",
        "config.step.pair.data.code": "Código de emparelhamento",
        "config.step.pair.data.name": "Nome do Home Assistant",
        "config.progress.pair_confirm": "A aguardar a sua confirmação no ecrã {device_name}…",
        "entity.text.display_text.name": "Texto apresentado",
    },
    "nl": {
        "config.step.pair.title": "Een Screensight-display koppelen",
        "config.step.pair.data.code": "Koppelingscode",
        "config.step.pair.data.name": "Naam van Home Assistant",
        "config.progress.pair_confirm": "Wachten op je bevestiging op het {device_name}-display…",
        "entity.text.display_text.name": "Weergegeven tekst",
    },
}


def supported_locales() -> list[str]:
    """Return every locale Home Assistant ships component translations for."""
    spec = importlib.util.find_spec("homeassistant")
    locations = list(spec.submodule_search_locations or ()) if spec else []
    if not locations:
        msg = "homeassistant is not importable; run inside the project environment"
        raise SystemExit(msg)

    locales: set[str] = set()
    for location in locations:
        components = Path(location) / "components"
        for translations in components.glob("*/translations"):
            locales.update(path.stem for path in translations.glob("*.json"))
    return sorted(locales)


def _apply_overrides(messages: dict, overrides: dict[str, str]) -> None:
    """Set ``messages[a][b]...`` for each dotted key in ``overrides``."""
    for dotted, value in overrides.items():
        node = messages
        keys = dotted.split(".")
        for key in keys[:-1]:
            node = node[key]
        node[keys[-1]] = value


def main() -> None:
    """Write one translation file per Home Assistant locale."""
    base = json.loads(SOURCE_FILE.read_text(encoding="utf-8"))
    locales = supported_locales()
    for locale in locales:
        messages = copy.deepcopy(base)
        _apply_overrides(messages, OVERRIDES.get(locale, {}))
        destination = TRANSLATIONS_DIR / f"{locale}.json"
        destination.write_text(
            json.dumps(messages, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
    print(f"Wrote {len(locales)} locale files to {TRANSLATIONS_DIR}", file=sys.stderr)


if __name__ == "__main__":
    main()
