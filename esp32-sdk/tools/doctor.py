#!/usr/bin/env python3
import argparse
import json
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def report(project: Path | None) -> dict:
    findings = []
    tools = {}
    for command in ("git", "cmake", "uv"):
        tools[command] = shutil.which(command)
        if tools[command] is None:
            findings.append(f"Install {command} before using the development helpers")
    toolchains = {}
    versions = json.loads((ROOT / "tools/toolchains.json").read_text())["tested_versions"]
    for version in versions:
        source = ROOT / ".deps" / f"esp-idf-v{version}"
        python = ROOT / ".deps/espressif/python_env" / f"bobe-{version}/bin/python"
        toolchains[version] = {"source": source.is_dir(), "python": python.is_file()}
    if not any(value["source"] and value["python"] for value in toolchains.values()):
        findings.append("Run tools/bootstrap.sh to prepare a project-local ESP-IDF environment")
    project_status = None
    if project is not None:
        project = project.expanduser().resolve()
        project_status = {"path": str(project), "cmake": (project / "CMakeLists.txt").is_file(),
                          "component_manifest": (project / "main/idf_component.yml").is_file(),
                          "private_credentials": (project / "private/bobe_credentials.h").is_file()}
        if not project_status["cmake"]:
            findings.append("Project CMakeLists.txt is missing")
    return {"sdk": "esp-bobe", "version": "0.2.0", "tools": tools, "toolchains": toolchains,
            "project": project_status, "findings": findings, "ok": not findings,
            "hardware_verified": False}


def main() -> None:
    parser = argparse.ArgumentParser(description="Read-only SDK environment and project diagnostics.")
    parser.add_argument("--project", type=Path)
    args = parser.parse_args()
    result = report(args.project)
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["ok"] else 1)


if __name__ == "__main__":
    main()
