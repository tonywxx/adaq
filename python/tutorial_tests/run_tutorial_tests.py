"""Small repository-owned parity gate for the offline M12 tutorial."""

from __future__ import annotations

import json
import pathlib
import tomllib
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[2]
EXAMPLES = ROOT / "examples/python"
FIXTURE = ROOT / "src-tauri/fixtures/python-tutorial"

EXPECTED_FIXTURE = {
    "fixtureId": "python-tutorial-a-share@1",
    "synthetic": True,
    "instrumentCount": 12,
    "sessionCount": 180,
    "instrumentSha256": "a6963ebf7e0481749a1db2db22ef2f23bc5fee6d39d5afe258ca27c3c17fdaca",
    "calendarSha256": "2e423b9b46a4af56729da0fee4298ed47cdaee70b6e0bc4e4e8f5fb03cd978a9",
    "barsSha256": "fd4dc3bcccb554ad29ca08e89c35c220dafcb546db4df436009612f795a2bb4e",
    "contentSha256": "6d44423e009d2251d442f388f1621242fc4dac1e0eb5d9b774fc62ecd135d848",
}

PROJECTS = {
    "py-factor-cross-sectional-momentum": {
        "kind": "factor",
        "parameter": ("lookback", "20", ["5", "20", "60"]),
        "readme": ("lookback={5,20,60}", "Synthetic Demonstration"),
        "readme_zh": ("lookback={5,20,60}", "合成演示"),
    },
    "py-model-qlib-ridge-return": {
        "kind": "model",
        "parameter": ("alpha", "1", ["0.1", "1", "10"]),
        "readme": ("alpha={0.1,1,10}", "Synthetic Demonstration"),
        "readme_zh": ("alpha={0.1,1,10}", "合成演示"),
    },
    "py-strategy-top-n-forecast": {
        "kind": "strategy",
        "parameter": ("top-n", "3", ["1", "3", "5"]),
        "readme": ("M13 continuation", "not executable"),
        "readme_zh": ("M13", "不可执行"),
    },
}


def assert_true(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def main() -> None:
    manifest = json.loads((FIXTURE / "manifest.json").read_text(encoding="utf-8"))
    assert_true(manifest == EXPECTED_FIXTURE, "fixture manifest drifted")
    fixture_readme = (FIXTURE / "README.md").read_text(encoding="utf-8").lower()
    assert_true("fictional" in fixture_readme, "fixture is not labelled fictional")
    assert_true("offline" in fixture_readme, "fixture is not labelled offline")
    assert_true("credential" in fixture_readme, "fixture credential boundary is undocumented")

    for project_id, expected in PROJECTS.items():
        project_root = EXAMPLES / project_id
        project = tomllib.loads((project_root / "adaq-project.toml").read_text())
        pyproject = tomllib.loads((project_root / "pyproject.toml").read_text())
        lock = tomllib.loads((project_root / "pylock.toml").read_text())
        assert_true(project["project-id"] == project_id, f"{project_id}: id mismatch")
        assert_true(project["kind"] == expected["kind"], f"{project_id}: kind mismatch")
        assert_true(project["license"] == "Apache-2.0", f"{project_id}: license mismatch")
        if project_id == "py-model-qlib-ridge-return":
            assert_true(
                project["adapter-id"] == "qlib-linear-ridge@1",
                f"{project_id}: adapter identity drift",
            )
        assert_true(project["runtime-profile"] == "adaq-python@1", f"{project_id}: runtime drift")
        assert_true(project["sdk-profile"] == "adaq-research-sdk@1", f"{project_id}: SDK drift")
        parameter_id, default, allowed = expected["parameter"]
        parameter = project["parameters"][0]
        assert_true(parameter["id"] == parameter_id, f"{project_id}: parameter id drift")
        assert_true(parameter["default"] == default, f"{project_id}: parameter default drift")
        assert_true(parameter["allowed-values"] == allowed, f"{project_id}: parameter grid drift")
        assert_true(pyproject["project"]["dependencies"] == [], f"{project_id}: dependency added")
        assert_true(lock["format"] == "adaq-lock-v1", f"{project_id}: lock format drift")
        assert_true(lock["platform"] == "managed", f"{project_id}: lock platform drift")
        assert_true((project_root / "LICENSE").read_text().startswith("Apache License"), f"{project_id}: license missing")
        for marker in expected["readme"]:
            assert_true(marker in (project_root / "README.md").read_text(encoding="utf-8"), f"{project_id}: English guide marker missing: {marker}")
        for marker in expected["readme_zh"]:
            assert_true(marker in (project_root / "README.zh-CN.md").read_text(encoding="utf-8"), f"{project_id}: Chinese guide marker missing: {marker}")
        source = (project_root / "src/project.py").read_text(encoding="utf-8").lower()
        for forbidden in ("provider", "requests", "urllib", "http://", "https://", "sqlite"):
            assert_true(forbidden not in source, f"{project_id}: forbidden source dependency {forbidden}")

    guide = (ROOT / "docs/m12-python-research-and-model-lab.md").read_text(encoding="utf-8")
    guide_zh = (ROOT / "docs/m12-python-research-and-model-lab.zh-CN.md").read_text(encoding="utf-8")
    acceptance = (ROOT / "docs/m12-python-research-manual-acceptance.md").read_text(encoding="utf-8")
    acceptance_zh = (ROOT / "docs/m12-python-research-manual-acceptance.zh-CN.md").read_text(encoding="utf-8")
    for document in (guide, guide_zh, acceptance, acceptance_zh):
        for marker in ("1–100", "101–105", "106–140", "141–145", "146–180"):
            assert_true(marker in document, f"tutorial window missing: {marker}")
        for project_id in PROJECTS:
            assert_true(project_id in document, f"tutorial project missing: {project_id}")
    assert_true("M13" in guide and "M14" in guide, "deferred milestone boundary missing")
    assert_true("Run Python Tutorial" in guide, "English tutorial surface missing")
    assert_true("Run Python Tutorial" in guide_zh, "Chinese tutorial surface missing")
    workflow = (ROOT / ".github/workflows/python-research.yml").read_text(encoding="utf-8")
    for platform in ("macos-14", "windows-latest", "ubuntu-latest"):
        assert_true(platform in workflow, f"supported platform missing: {platform}")
    assert_true("tutorial_golden_contracts_cover_fixture_windows_and_model_boundaries" in workflow, "tutorial Golden gate missing")

    archives = [
        path
        for path in EXAMPLES.rglob("*")
        if path.is_file() and path.suffix.lower() in {".adaq", ".zip"}
    ]
    for archive_path in archives:
        with zipfile.ZipFile(archive_path) as archive:
            names = "\n".join(archive.namelist())
            assert_true("python-tutorial-a-share" not in names, f"fixture embedded in {archive_path}")

    print(f"tutorial parity passed: fixture=1 projects={len(PROJECTS)} bilingual_docs=4 archives={len(archives)}")


if __name__ == "__main__":
    main()
