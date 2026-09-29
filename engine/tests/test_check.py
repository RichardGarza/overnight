"""`check` on a bare install: valid JSON in the spec shape, exit 0, no downloads."""

import json
import subprocess
import sys

from overnight_engine import check, paths


def test_gather_shape_on_bare_models_dir(scratch_models):
    out = check.gather()
    for key in ("python", "torch", "device", "acestep", "demucs", "rvc", "ffmpeg", "models", "problems"):
        assert key in out
    assert out["device"] in ("mps", "cpu")
    assert set(out["models"]) == {"acestep", "demucs", "rvcAssets"}
    # Nothing is downloaded on a bare models folder, so nothing is present.
    assert out["models"] == {"acestep": False, "demucs": False, "rvcAssets": False}
    assert isinstance(out["problems"], list)
    assert out["modelsDir"] == str(scratch_models)
    # And check never wrote anything into the models folder.
    assert not any(p.is_file() for p in scratch_models.rglob("*"))


def test_check_subprocess_is_single_json_line_and_exit_zero(child_env):
    cp = subprocess.run(
        [sys.executable, "-m", "overnight_engine", "check"],
        capture_output=True, text=True, env=child_env, timeout=120,
    )
    assert cp.returncode == 0, cp.stderr
    lines = [l for l in cp.stdout.splitlines() if l.strip()]
    assert len(lines) == 1, cp.stdout
    obj = json.loads(lines[0])
    assert obj["python"].startswith("3.")
    assert obj["models"]["acestep"] is False


def test_models_missing_is_reported_as_problem(scratch_models):
    out = check.gather()
    if out["acestep"]:
        assert any("ACE-Step weights" in p for p in out["problems"])
    if out["torch"] is None:
        assert any("torch" in p for p in out["problems"])


def test_find_ffmpeg_honours_override(monkeypatch, tmp_path):
    fake = tmp_path / "ffmpeg"
    fake.write_text("#!/bin/sh\n")
    fake.chmod(0o755)
    monkeypatch.setenv("OVERNIGHT_FFMPEG", str(fake))
    assert paths.find_ffmpeg() == str(fake)


def test_has_package_checks_site_packages(tmp_path):
    venv = tmp_path / "venv-rvc"
    sp = venv / "lib" / "python3.10" / "site-packages"
    sp.mkdir(parents=True)
    assert not paths.has_package(venv, "rvc_python")
    (sp / "rvc_python").mkdir()
    assert paths.has_package(venv, "rvc_python")
