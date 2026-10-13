#!/usr/bin/env python3
"""Create a small consumer project without copying SDK sources or private data."""

from __future__ import annotations

import argparse
import json
import re
import shutil
from pathlib import Path

SDK_ROOT = Path(__file__).resolve().parent.parent
TARGETS = ("esp32", "esp32c3", "esp32c6", "esp32s3")
TEMPLATES = {"text-terminal": "text_terminal", "sensor-node": "sensor_node", "protocol": "protocol", "wb12": "wb12"}


def create_project(
    name: str,
    output: Path,
    template: str = "text-terminal",
    target: str | None = None,
    sdk_root: Path = SDK_ROOT,
    git_url: str | None = None,
    git_ref: str | None = None,
) -> Path:
    if re.fullmatch(r"[a-z][a-z0-9_]{0,30}", name) is None:
        raise ValueError("project name must contain 1–31 lowercase letters, digits, or underscores")
    if template not in TEMPLATES:
        raise ValueError("unknown project template")
    target = target or ("esp32s3" if template == "wb12" else "esp32c3")
    if target not in TARGETS or (template == "wb12" and target != "esp32s3"):
        raise ValueError("the selected template does not support this target")
    if (git_url is None) != (git_ref is None):
        raise ValueError("Git consumption requires both --git and --ref")
    if git_url is not None and template == "wb12":
        raise ValueError("the WB-12 reference application currently uses a local SDK checkout")
    output = output.expanduser().absolute()
    if output.exists() or output.is_symlink():
        raise ValueError("output already exists; existing projects are never overwritten")
    source = sdk_root.resolve() / "examples" / TEMPLATES[template]
    required = ["CMakeLists.txt", "main/CMakeLists.txt", "main/main.c", "sdkconfig.defaults"]
    if template == "sensor-node":
        required.append("main/Kconfig.projbuild")
    if template == "wb12":
        required[2] = "main/app_main.c"
        required += ["partitions.csv", "sdkconfig.provisioning.defaults", "sdkconfig.edge.defaults"]
    for relative in required:
        path = source / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"template file is unavailable: {relative}")
    output.mkdir(parents=True, exist_ok=False)
    try:
        for relative in required:
            destination = output / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / relative, destination)
        cmake = (output / "CMakeLists.txt").read_text()
        cmake = re.sub(r"project\([^)]+\)", f"project({name})", cmake)
        cmake = cmake.replace(
            "cmake_minimum_required(VERSION 3.24)",
            f'cmake_minimum_required(VERSION 3.24)\nset(IDF_TARGET "{target}" CACHE STRING "ESP32 target")',
        )
        if template == "wb12":
            cmake = cmake.replace(
                "cmake_minimum_required(VERSION 3.20)",
                f'cmake_minimum_required(VERSION 3.20)\nset(ESP_BOBE_SDK_PATH {json.dumps(str(sdk_root.resolve()))})',
            )
            board_files = {
                "bobe_board_port": ["CMakeLists.txt", "bobe_board.c", "bobe_board_audio.c", "bobe_board_display.c", "bobe_board_key.c", "bobe_battery.c", "include/bobe_battery.h", "update-signing-public.pem"],
                "bobe_device_config": ["CMakeLists.txt", "include/bobe_device_config.h"],
            }
            for component, paths in board_files.items():
                src = source / "components" / component
                for relative in paths:
                    path = src / relative
                    if path.is_symlink() or not path.is_file():
                        raise ValueError("board template must not contain symbolic links")
                    destination = output / "components" / component / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(path, destination)
        else:
            component = "bobe_protocol" if template == "protocol" else "bobe_client"
            dependency = (
                {"git": git_url, "path": f"esp32-sdk/components/{component}", "version": git_ref}
                if git_url is not None
                else {"path": str(sdk_root.resolve() / "components" / component)}
            )
            (output / "main/idf_component.yml").write_text(
                json.dumps({"dependencies": {component: dependency}}, indent=2) + "\n"
            )
        (output / "CMakeLists.txt").write_text(cmake)
        (output / ".gitignore").write_text(
            "build/\nbuild-*/\nmanaged_components/\nsdkconfig\nsdkconfig.old\nprivate/\n.venv/\n__pycache__/\n"
        )
        (output / "README.md").write_text(
            f"# {name}\n\nGenerated from the BoBe ESP32 SDK's `{template}` template for `{target}`.\n\n"
            "Use ESP-IDF 6.1 (or the tested 6.0.2 baseline).\n\n"
            "```sh\nidf.py menuconfig\nidf.py build\n```\n\n"
            "Building does not flash hardware. Keep device credentials under the ignored `private/` directory.\n"
        )
    except Exception:
        shutil.rmtree(output)
        raise
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--template", choices=tuple(TEMPLATES), default="text-terminal")
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--sdk", type=Path, default=SDK_ROOT)
    parser.add_argument("--git", dest="git_url", help="BoBe repository URL for pinned ESP32 SDK components")
    parser.add_argument("--ref", dest="git_ref", help="Git commit or tag to pin")
    args = parser.parse_args()
    try:
        result = create_project(args.name, args.output or Path(args.name), args.template, args.target, args.sdk, args.git_url, args.git_ref)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(f"Created {result}")
    print(f"Build with: idf.py -C {result} build")


if __name__ == "__main__":
    main()
