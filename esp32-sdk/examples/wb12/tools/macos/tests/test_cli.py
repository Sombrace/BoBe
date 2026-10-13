#!/usr/bin/env python3

import hashlib
import json
import os
import shutil
import stat
import struct
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
WRAPPER = ROOT / "tools" / "bobe_capture.sh"
WORK = ROOT / ".tmp" / "bobe-capture-tests"
GENERATOR = ROOT / ".tmp" / "bobe-capture-fixture-generator"


def run(*arguments: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(WRAPPER), *(str(value) for value in arguments)],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )


def parsed_error(result: subprocess.CompletedProcess[str], code: str) -> None:
    assert result.returncode != 0, result.stdout
    assert result.stdout == "", result.stdout
    payload = json.loads(result.stderr)
    assert payload["ok"] is False
    assert payload["error"]["code"] == code, payload


def png_dimensions(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    assert data[12:16] == b"IHDR"
    return struct.unpack(">II", data[16:24])


def cleanup() -> None:
    if GENERATOR.exists() or GENERATOR.is_symlink():
        GENERATOR.unlink()
    if WORK.exists():
        shutil.rmtree(WORK)


def main() -> None:
    if WORK.exists():
        for entry in WORK.iterdir():
            if entry.is_symlink() or entry.is_file():
                entry.unlink()
            else:
                raise AssertionError(f"unexpected test directory entry: {entry}")
    else:
        WORK.mkdir(parents=True)

    help_result = run("--help")
    assert help_result.returncode == 0, help_result.stderr
    for command in ("list", "preflight", "authorize", "frame", "ocr", "rectify"):
        assert command in help_result.stdout
    assert "Invoke through tools/bobe_capture.sh" in help_result.stdout
    assert "All commands emit one JSON object" in help_result.stdout

    frame_help = run("frame", "--help")
    assert frame_help.returncode == 0, frame_help.stderr
    assert "--camera-id" in frame_help.stdout
    assert "--warmup-ms 250..10000" in frame_help.stdout

    parsed_error(run(), "INVALID_ARGUMENT")
    list_without_json = run("list")
    assert list_without_json.returncode == 0, list_without_json.stderr
    assert json.loads(list_without_json.stdout)["command"] == "list"
    summary = run("list", "--summary")
    assert summary.returncode == 0, summary.stderr
    summary_value = json.loads(summary.stdout)
    assert summary_value["summary"] is True
    for device in summary_value["devices"]:
        assert "formats" not in device
        assert device["format_count"] > 0
    parsed_error(run("authorize", "--json"), "INVALID_ARGUMENT")
    parsed_error(run("authorize", "--camera-id", "unused"), "INVALID_ARGUMENT")
    parsed_error(run("ocr", "--input", WORK / "missing.png"), "INPUT_NOT_FOUND")
    parsed_error(
        run(
            "frame",
            "--camera-id",
            "missing",
            "--output",
            WORK / "out.png",
            "--warmup-ms",
            "10001",
        ),
        "INVALID_ARGUMENT",
    )

    ocr_fixture = WORK / "ocr.png"
    rectangle_fixture = WORK / "rectangle.png"
    blank_fixture = WORK / "blank.png"
    subprocess.run(
        [
            "xcrun",
            "swiftc",
            str(ROOT / "tools" / "macos" / "tests" / "GenerateFixtures.swift"),
            "-o",
            str(GENERATOR),
            "-framework",
            "CoreGraphics",
            "-framework",
            "CoreText",
            "-framework",
            "ImageIO",
        ],
        cwd=ROOT,
        check=True,
    )
    subprocess.run(
        [
            str(GENERATOR),
            str(ocr_fixture),
            str(rectangle_fixture),
            str(blank_fixture),
        ],
        cwd=ROOT,
        check=True,
    )

    symlink = WORK / "linked.png"
    os.symlink(ocr_fixture.name, symlink)
    parsed_error(run("ocr", "--input", symlink), "UNSAFE_INPUT")

    ocr = run("ocr", "--input", ocr_fixture)
    assert ocr.returncode == 0, ocr.stderr
    ocr_payload = json.loads(ocr.stdout)
    recognized = " ".join(item["text"] for item in ocr_payload["results"])
    assert "BOBE" in recognized and "1234" in recognized, recognized
    assert ocr_payload["compute_device"] == "cpu"
    assert ocr_payload["request_revision"] == 3

    parsed_error(
        run(
            "rectify",
            "--input",
            blank_fixture,
            "--output",
            WORK / "blank-rectified.png",
            "--width",
            "320",
            "--height",
            "200",
        ),
        "NO_HIGH_CONFIDENCE_RECTANGLE",
    )

    output_symlink = WORK / "output-link.png"
    os.symlink("not-created.png", output_symlink)
    parsed_error(
        run(
            "rectify",
            "--input",
            rectangle_fixture,
            "--output",
            output_symlink,
            "--width",
            "320",
            "--height",
            "200",
        ),
        "OUTPUT_EXISTS",
    )

    rectified = WORK / "rectified.png"
    parsed_error(
        run(
            "rectify",
            "--input",
            rectangle_fixture,
            "--output",
            WORK / "missing-dimensions.png",
        ),
        "INVALID_ARGUMENT",
    )
    rectify = run(
        "rectify",
        "--input",
        rectangle_fixture,
        "--output",
        rectified,
        "--width",
        "280",
        "--height",
        "240",
    )
    assert rectify.returncode == 0, rectify.stderr
    rectify_payload = json.loads(rectify.stdout)
    assert rectify_payload["compute_device"] == "cpu"
    assert rectify_payload["width"] == 280
    assert rectify_payload["height"] == 240
    assert png_dimensions(rectified) == (280, 240)
    assert rectify_payload["sha256"] == hashlib.sha256(rectified.read_bytes()).hexdigest()
    assert stat.S_IMODE(rectified.stat().st_mode) == 0o600

    parsed_error(
        run(
            "rectify",
            "--input",
            rectangle_fixture,
            "--output",
            rectified,
        ),
        "OUTPUT_EXISTS",
    )
    cleanup()
    print("bobe-capture offline tests passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        cleanup()
        print(f"bobe-capture offline tests failed: {error}", file=sys.stderr)
        raise
