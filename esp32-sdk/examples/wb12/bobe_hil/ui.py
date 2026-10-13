import subprocess
from pathlib import Path

from bobe_hil.evidence import EvidenceRun
from bobe_hil.media import rgb565_to_png
from bobe_hil.paths import REPOSITORY_ROOT


UI_SOURCE = REPOSITORY_ROOT / "host_tests" / "ui"
UI_BUILD = REPOSITORY_ROOT / ".tmp" / "hil-ui"
UI_EXECUTABLE = UI_BUILD / "bobe_ui_visual_tests"
UI_REFERENCES = UI_SOURCE / "references.sha256"
UI_WIDTH = 280
UI_HEIGHT = 240


def reference_case_count() -> int:
    if not UI_REFERENCES.is_file() or UI_REFERENCES.is_symlink():
        return 0
    return len(
        [
            line
            for line in UI_REFERENCES.read_text(encoding="ascii").splitlines()
            if line.strip()
        ]
    )


def checked_command(command: list[str], log_path: Path) -> None:
    result = subprocess.run(
        command,
        cwd=REPOSITORY_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    log_path.write_text(
        result.stdout + result.stderr,
        encoding="utf-8",
    )
    log_path.chmod(0o600)
    if result.returncode != 0:
        raise RuntimeError(
            f"{command[0]} failed with exit code {result.returncode}; "
            f"see {log_path}"
        )


def render_ui_step(run: EvidenceRun) -> dict[str, object]:
    working = run.directory / "working"
    working.mkdir(mode=0o700)
    configure_log = working / "configure.log"
    build_log = working / "build.log"
    compare_log = working / "compare.log"
    render_log = working / "render.log"
    rendered = working / "rendered"
    rendered.mkdir(mode=0o700)
    run.event("ui.build.started")
    UI_BUILD.mkdir(mode=0o700, parents=True, exist_ok=True)
    checked_command(
        [
            "cmake",
            "-S",
            str(UI_SOURCE),
            "-B",
            str(UI_BUILD),
            "-DCMAKE_BUILD_TYPE=Debug",
        ],
        configure_log,
    )
    checked_command(
        ["cmake", "--build", str(UI_BUILD), "--parallel"],
        build_log,
    )
    checked_command(
        [
            "ctest",
            "--test-dir",
            str(UI_BUILD),
            "--output-on-failure",
        ],
        compare_log,
    )
    checked_command(
        [str(UI_EXECUTABLE), str(rendered)],
        render_log,
    )
    raw_files = sorted(rendered.glob("*.rgb565"))
    if len(raw_files) != reference_case_count():
        raise RuntimeError("UI renderer produced an incomplete visual set")
    for raw_path in raw_files:
        png_path = raw_path.with_suffix(".png")
        rgb565_to_png(
            raw_path,
            png_path,
            width=UI_WIDTH,
            height=UI_HEIGHT,
        )
        run.add_artifact(raw_path, "ui-rgb565")
        run.add_artifact(png_path, "ui-png")
    for log_path in (configure_log, build_log, compare_log, render_log):
        run.add_artifact(log_path, "ui-log")
    run.event("ui.render.completed", cases=len(raw_files))
    return {
        "case_count": len(raw_files),
        "artifact_count": len(run.manifest["artifacts"]),
    }


def render_ui_evidence() -> dict[str, object]:
    run = EvidenceRun.create("ui-render", "logical")
    try:
        result = render_ui_step(run)
        run.finish("passed")
        return {
            "run_id": run.manifest["run_id"],
            "directory": str(run.directory),
            "status": run.manifest["status"],
            **result,
        }
    except (OSError, RuntimeError, ValueError) as error:
        run.finish("failed", str(error))
        raise
    finally:
        run.close()
