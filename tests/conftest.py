"""Global fixtures for the Screensight test-suite."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))


@pytest.fixture(autouse=True)
def auto_enable_custom_integrations(enable_custom_integrations):
    """Load the integration from ``custom_components/`` for every test."""
    return


@pytest.fixture(autouse=True)
def auto_mock_zeroconf(mock_async_zeroconf):
    """Keep the ``zeroconf`` dependency from opening real mDNS sockets."""
    return
