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
    pin = dict(
        line.split("=", 1)
        for line in (ROOT / "tools/esp-idf.lock").read_text().splitlines()
        if "=" in line
    )
    source = ROOT / ".deps" / f"esp-idf-{pin['ESP_IDF_DESCRIBE']}"
    python = ROOT / ".deps/espressif/python_env" / f"bobe-{pin['ESP_IDF_VERSION']}/bin/python"
    toolchains = {
        pin["ESP_IDF_DESCRIBE"]: {
            "commit": pin["ESP_IDF_COMMIT"],
            "source": (source / "export.sh").is_file(),
            "python": python.is_file(),
        }
    }
    if not all(value["source"] and value["python"] for value in toolchains.values()):
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
