import os
import secrets
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any

from bobe_hil.evidence import EvidenceRun
from bobe_hil.live_backend import LiveScenarioBackend
from bobe_hil.mock_backend import MockScenarioBackend
from bobe_hil.readiness import readiness_for
from bobe_hil.scenario import Scenario, ScenarioStep, scenario_contract_sha256


STATUS_EXIT_CODES = {
    "passed": 0,
    "planned": 0,
    "failed": 1,
    "blocked": 3,
}
EXECUTION_MODES = {"dry-run", "live", "mock"}


def result_for(run: EvidenceRun, **extra: Any) -> dict[str, Any]:
    status = str(run.manifest["status"])
    return {
        "schema_version": 1,
        "run_id": run.manifest["run_id"],
        "scenario": run.manifest["scenario"],
        "claim": run.manifest["claim"],
        "target_claim": run.manifest.get("target_claim"),
        "execution_mode": run.manifest["execution_mode"],
        "status": status,
        "reason": run.manifest.get("reason"),
        "directory": str(run.directory),
        "artifact_count": len(run.manifest["artifacts"]),
        "exit_code": STATUS_EXIT_CODES[status],
        **extra,
    }


def rendered_step(step: ScenarioStep) -> dict[str, Any]:
    return {
        "step_id": step.step_id,
        "operation": step.operation,
        "parameters": step.parameters,
    }


def execute_cleanup(
    scenario: Scenario,
    backend: MockScenarioBackend | LiveScenarioBackend,
    run: EvidenceRun,
) -> None:
    """Run cleanup and verify that physical state was released."""

    errors: list[str] = []
    for step in scenario.cleanup:
        run.event(
            "cleanup.started",
            step_id=step.step_id,
            operation=step.operation,
        )
        try:
            result = backend.execute(step)
        except (OSError, RuntimeError, TimeoutError, ValueError) as error:
            errors.append(f"{step.step_id}: {error}")
            run.event(
                "cleanup.failed",
                step_id=step.step_id,
                operation=step.operation,
                error=str(error),
            )
        else:
            run.event(
                "cleanup.completed",
                step_id=step.step_id,
                operation=step.operation,
                result=result,
            )
    forced_cleanup = backend.finalize()
    state = backend.state()
    run.event(
        "cleanup.finalized",
        state=state,
        forced_cleanup=forced_cleanup,
    )
    if errors or forced_cleanup or not state["fixture_released"]:
        raise RuntimeError(
            "scenario cleanup failed"
            + (f": {'; '.join(errors)}" if errors else "")
            + (
                f"; forced: {', '.join(forced_cleanup)}"
                if forced_cleanup
                else ""
            )
        )


def run_scenario(
    scenario: Scenario,
    *,
    execution_mode: str,
) -> dict[str, Any]:
    if execution_mode not in EXECUTION_MODES:
        raise ValueError("scenario execution mode is invalid")
    run = EvidenceRun.create(
        scenario.name,
        "simulation" if execution_mode == "mock" else scenario.claim,
        target_claim=scenario.claim if execution_mode == "mock" else None,
        scenario_contract_sha256=scenario_contract_sha256(scenario),
    )
    run.manifest["execution_mode"] = execution_mode
    try:
        requirement_status = (
            readiness_for(scenario.requires)
            if execution_mode == "live"
            else {}
        )
        run.event(
            "run.started",
            execution_mode=execution_mode,
            claim=run.manifest["claim"],
            target_claim=run.manifest.get("target_claim"),
            requirements=requirement_status,
            scenario_contract_sha256=run.manifest["scenario_contract_sha256"],
        )
        if execution_mode == "dry-run":
            plan = {
                "steps": [rendered_step(step) for step in scenario.steps],
                "cleanup": [rendered_step(step) for step in scenario.cleanup],
            }
            for step in scenario.steps:
                run.event("step.planned", **rendered_step(step))
            for step in scenario.cleanup:
                run.event("cleanup.planned", **rendered_step(step))
            run.finish("planned", "dry-run-did-not-actuate-hardware")
            return result_for(run, plan=plan)

        if execution_mode == "live":
            unavailable = {
                name: value
                for name, value in requirement_status.items()
                if value["state"] != "available"
            }
            if unavailable:
                run.event("run.preflight.unavailable", requirements=unavailable)
                run.finish("blocked", "scenario-hardware-unavailable")
                return result_for(run, requirements=unavailable)

        backend: MockScenarioBackend | LiveScenarioBackend = (
            MockScenarioBackend(run)
            if execution_mode == "mock"
            else LiveScenarioBackend(run, scenario)
        )
        execution_error: Exception | None = None
        try:
            for step in scenario.steps:
                run.event(
                    "step.started",
                    step_id=step.step_id,
                    operation=step.operation,
                )
                step_result = backend.execute(step)
                run.event(
                    "step.completed",
                    step_id=step.step_id,
                    operation=step.operation,
                    result=step_result,
                )
        except (OSError, RuntimeError, TimeoutError, ValueError) as error:
            execution_error = error
        try:
            execute_cleanup(scenario, backend, run)
        except (OSError, RuntimeError, TimeoutError, ValueError) as error:
            if execution_error is None:
                execution_error = error
            else:
                execution_error = RuntimeError(f"{execution_error}; cleanup: {error}")
        if execution_error is not None:
            raise execution_error
        run.finish("passed")
        return result_for(run)
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        if run.manifest.get("status") == "running":
            run.finish("failed", str(error))
        return result_for(run)
    finally:
        run.close()


def write_junit(result: dict[str, Any], output: Path) -> None:
    if output.is_symlink():
        raise ValueError("JUnit output must not be a symbolic link")
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    status = result["status"]
    suite = ET.Element(
        "testsuite",
        {
            "name": "bobe-hil",
            "tests": "1",
            "failures": "1" if status == "failed" else "0",
            "skipped": "1" if status == "blocked" else "0",
        },
    )
    case = ET.SubElement(
        suite,
        "testcase",
        {
            "classname": "bobe.hil.scenario",
            "name": str(result["scenario"]),
        },
    )
    if status == "failed":
        failure = ET.SubElement(
            case,
            "failure",
            {"message": str(result.get("reason") or "scenario failed")},
        )
        failure.text = str(result.get("directory"))
    elif status == "blocked":
        ET.SubElement(
            case,
            "skipped",
            {"message": str(result.get("reason") or "scenario blocked")},
        )
    properties = ET.SubElement(suite, "properties")
    for name in (
        "run_id",
        "claim",
        "target_claim",
        "execution_mode",
        "directory",
    ):
        if result.get(name) is None:
            continue
        ET.SubElement(
            properties,
            "property",
            {"name": name, "value": str(result[name])},
        )
    rendered = ET.tostring(suite, encoding="utf-8", xml_declaration=True)
    temporary = output.with_name(f".{output.name}.{secrets.token_hex(8)}.tmp")
    descriptor = os.open(
        temporary,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL,
        0o600,
    )
    try:
        with os.fdopen(descriptor, "wb") as destination:
            destination.write(rendered)
            destination.write(b"\n")
            destination.flush()
            os.fsync(destination.fileno())
        os.replace(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)
