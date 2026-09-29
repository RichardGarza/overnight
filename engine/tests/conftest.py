"""Make the package importable when pytest is run from anywhere, and give
tests a scratch models folder so nothing touches the real one."""

import os
import sys
from pathlib import Path

import pytest

ENGINE_DIR = Path(__file__).resolve().parent.parent
if str(ENGINE_DIR) not in sys.path:
    sys.path.insert(0, str(ENGINE_DIR))

FIXTURES = Path(__file__).resolve().parent / "fixtures"


@pytest.fixture
def fixture_song() -> dict:
    import json

    return json.loads((FIXTURES / "song.json").read_text())


@pytest.fixture
def scratch_models(tmp_path, monkeypatch):
    """OVERNIGHT_MODELS pointed at an empty temp dir (a bare install)."""
    m = tmp_path / "models"
    m.mkdir()
    monkeypatch.setenv("OVERNIGHT_MODELS", str(m))
    monkeypatch.setenv("OVERNIGHT_DATA", str(tmp_path))
    return m


@pytest.fixture
def child_env(scratch_models):
    """Environment for running the engine in a subprocess."""
    env = dict(os.environ)
    env["OVERNIGHT_MODELS"] = str(scratch_models)
    env["OVERNIGHT_DATA"] = str(scratch_models.parent)
    env["PYTHONPATH"] = str(ENGINE_DIR) + os.pathsep + env.get("PYTHONPATH", "")
    env["PYTHONUNBUFFERED"] = "1"
    return env
