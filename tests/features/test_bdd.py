"""BDD entry points for the Screensight feature files."""

from __future__ import annotations

import pytest
from pytest_bdd import scenarios

pytestmark = pytest.mark.bdd

scenarios("pairing.feature")
scenarios("display_text.feature")
