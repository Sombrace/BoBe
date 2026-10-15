import asyncio
import json
import subprocess
import threading
from typing import Any

from bobe_hil import device, fixture, network
from bobe_hil.bodylink import load_private_config, serve_once
from bobe_hil.calibration import (
    load_audio_calibration,
    load_camera_calibration,
)
from bobe_hil.evidence import EvidenceRun
from bobe_hil.media import (
    audio_challenge,
    audio_metrics,
    play_record_audio,
    record_audio,
    write_metrics,
    write_wav,
)
from bobe_hil.paths import REPOSITORY_ROOT
from bobe_hil.paths import exclusive_device_access
from bobe_hil.scenario import Scenario, ScenarioStep
from bobe_hil.ui import render_ui_step


OPERATION_HANDLERS = {
    "bodylink_stop": "bodylink_stop",
    "bodylink_stub": "bodylink_stub",
    "capture_audio": "capture_audio",
    "capture_frame": "capture_frame",
    "capture_serial": "capture_serial",
    "fixture_chord": "fixture_chord",
    "fixture_press": "fixture_press",
    "fixture_release_all": "fixture_release_all",
    "fixture_reset": "fixture_reset",
    "network_profile": "apply_network_profile",
    "ui_golden": "ui_golden",
}


class BodyLinkService:
    def __init__(self, scenario: str, timeout: float, run: EvidenceRun) -> None:
        self.scenario = scenario
        self.timeout = timeout
        self.run = run
        self.error: BaseException | None = None
        self.thread = threading.Thread(target=self._serve, daemon=True)

    def _serve(self) -> None:
        try:
            config = load_private_config(
                self.scenario,
                str(self.run.manifest["nonce"]),
            )
            asyncio.run(serve_once(config, self.run, timeout=self.timeout))
        except BaseException as error:
            self.error = error

    def start(self) -> None:
        self.thread.start()

    def join(self, timeout: float) -> bool:
        self.thread.join(timeout=timeout)
        return not self.thread.is_alive()


class LiveScenarioBackend:
    def __init__(
        self,
        run: EvidenceRun,
        scenario: Scenario | None = None,
        identity: str | None = None,
    ) -> None:
        self.run = run
        self.working = run.directory / "live-working"
        self.working.mkdir(mode=0o700)
        self.identity = identity
        self.monitor: device.SerialMonitor | None = None
        self.fixture_used = False
        self.fixture_actuated = False
        self.network_lease: dict[str, Any] | None = None
        self.services: dict[str, BodyLinkService] = {}
        self.device_access: Any | None = None
        if scenario is not None and any(
            step.operation == "capture_serial"
            for step in (*scenario.steps, *scenario.cleanup)
        ):
            try:
                self.start_serial_monitor()
            except (OSError, RuntimeError, ValueError) as error:
                self.run.event(
                    "live.serial.monitor.unavailable",
                    error=str(error),
                )

    def start_serial_monitor(self) -> None:
        if self.monitor is not None:
            return
        self.acquire_device_access()
        identity = self.identity or device.resolve_port()[0]
        self.identity = identity
        self.monitor = device.SerialMonitor(identity).start()
        self.run.event("live.serial.monitor.started", mac=identity)

    def acquire_device_access(self) -> None:
        if self.device_access is not None:
            return
        access = exclusive_device_access("device")
        access.__enter__()
        self.device_access = access

    def execute(self, step: ScenarioStep) -> dict[str, Any]:
        name = OPERATION_HANDLERS.get(step.operation)
        if name is None:
            raise RuntimeError(
                f"live operation has no local implementation: {step.operation}"
            )
        handler: Any = getattr(self, name)
        return handler(step)

    def ui_golden(self, _step: ScenarioStep) -> dict[str, Any]:
        return render_ui_step(self.run)

    def fixture_reset(self, _step: ScenarioStep) -> dict[str, Any]:
        self.acquire_device_access()
        self.start_serial_monitor()
        self.fixture_used = True
        self.fixture_actuated = True
        result = fixture.reset()
        self.fixture_actuated = False
        return result

    def fixture_press(self, step: ScenarioStep) -> dict[str, Any]:
        self.acquire_device_access()
        self.fixture_used = True
        self.fixture_actuated = True
        result = fixture.press(
            str(step.parameters["control"]),
            int(step.parameters["duration_ms"]),
        )
        self.fixture_actuated = False
        return result

    def fixture_chord(self, step: ScenarioStep) -> dict[str, Any]:
        self.acquire_device_access()
        self.fixture_used = True
        self.fixture_actuated = True
        result = fixture.chord(
            str(step.parameters["chord"]),
            int(step.parameters["duration_ms"]),
        )
        self.fixture_actuated = False
        return result

    def fixture_release_all(self, _step: ScenarioStep) -> dict[str, Any]:
        self.acquire_device_access()
        self.fixture_used = True
        result = fixture.release_all()
        self.fixture_actuated = False
        return result

    def apply_network_profile(self, step: ScenarioStep) -> dict[str, Any]:
        profile = str(step.parameters["profile"])
        ttl_seconds = int(step.parameters["ttl_seconds"])
        result = network.apply(profile, ttl_seconds)
        self.network_lease = result.get("lease") if profile != "clean" else None
        return result

    def capture_serial(self, step: ScenarioStep) -> dict[str, Any]:
        self.start_serial_monitor()
        expect = tuple(str(value) for value in step.parameters["expect"])
        reject = tuple(str(value) for value in step.parameters["reject"])
        monitor = self.monitor
        if monitor is None:
            raise RuntimeError("device serial monitor is unavailable")
        result = device.observe(
            seconds=float(step.parameters["duration_seconds"]),
            expect=expect,
            reject=reject,
            monitor=monitor,
            retain=False,
        )
        path = self.working / f"{step.step_id}.log"
        path.write_bytes(monitor.snapshot())
        path.chmod(0o600)
        artifact = self.run.add_artifact(path, "serial", f"{step.step_id}.log")
        if not result["passed"]:
            raise RuntimeError(
                f"{step.step_id}: expected serial markers were not observed"
                + (f"; rejected: {', '.join(result['rejected'])}" if result["rejected"] else "")
            )
        return {**result, "artifact": artifact}

    def bodylink_stub(self, step: ScenarioStep) -> dict[str, Any]:
        service = BodyLinkService(
            str(step.parameters["scenario"]),
            float(step.parameters["timeout_seconds"]),
            self.run,
        )
        service.start()
        self.services[step.step_id] = service
        if not bool(step.parameters["background"]):
            return self.stop_services()
        return {
            "service_id": step.step_id,
            "background": True,
            "scenario": step.parameters["scenario"],
        }

    def bodylink_stop(self, _step: ScenarioStep) -> dict[str, Any]:
        return self.stop_services()

    def stop_services(self) -> dict[str, Any]:
        stopped: list[str] = []
        errors: list[str] = []
        for name, service in sorted(self.services.items()):
            if not service.join(service.timeout + 5.0):
                errors.append(f"{name}: BodyLink peer did not stop")
            elif service.error is not None:
                errors.append(f"{name}: {service.error}")
            else:
                stopped.append(name)
        self.services.clear()
        if errors:
            raise RuntimeError("; ".join(errors))
        return {"stopped": stopped}

    def capture_audio(self, step: ScenarioStep) -> dict[str, Any]:
        calibration = load_audio_calibration()
        direction = str(step.parameters["direction"])
        duration = float(step.parameters["duration_seconds"])
        sample_rate = int(calibration["sample_rate"])
        capture_path = self.working / f"{step.step_id}.wav"
        artifacts: list[dict[str, Any]] = []
        metrics: dict[str, Any] | None = None
        if direction == "microphone":
            stimulus_path = self.working / f"{step.step_id}-stimulus.wav"
            write_wav(
                stimulus_path,
                audio_challenge(
                    str(self.run.manifest["nonce"]),
                    sample_rate,
                    duration,
                ),
                sample_rate,
            )
            capture = play_record_audio(
                str(calibration["input_device_name"]),
                str(calibration["output_device_name"]),
                stimulus_path,
                capture_path,
                1.0,
            )
            metrics = audio_metrics(stimulus_path, capture_path)
            metrics_path = self.working / f"{step.step_id}-metrics.json"
            write_metrics(metrics_path, metrics)
            artifacts.append(self.run.add_artifact(stimulus_path, "audio-stimulus"))
            artifacts.append(self.run.add_artifact(metrics_path, "audio-metrics"))
        else:
            capture = record_audio(
                str(calibration["input_device_name"]),
                capture_path,
                duration,
                sample_rate,
            )
        artifacts.append(self.run.add_artifact(capture_path, "audio-capture"))
        self.run.event(
            "live.audio.captured",
            step_id=step.step_id,
            direction=direction,
            duration_seconds=duration,
        )
        return {
            "direction": direction,
            "capture": capture,
            "metrics": metrics,
            "thresholds": calibration["thresholds"],
            "artifacts": artifacts,
        }

    def capture_frame(self, step: ScenarioStep) -> dict[str, Any]:
        calibration = load_camera_calibration()
        output = self.working / f"{step.step_id}.png"
        capture = subprocess.run(
            [
                str(REPOSITORY_ROOT / "tools" / "bobe_capture.sh"),
                "frame",
                "--camera-id",
                str(calibration["camera_id"]),
                "--output",
                str(output),
            ],
            cwd=REPOSITORY_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=180.0,
        )
        if capture.returncode != 0 or not output.is_file():
            raise RuntimeError(
                "camera frame capture failed: "
                + (capture.stderr.strip() or capture.stdout.strip())
            )
        try:
            value = json.loads(capture.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError("camera frame capture output is not JSON") from error
        artifact = self.run.add_artifact(output, "frame", f"{step.step_id}.png")
        return {
            "state": step.parameters["state"],
            "camera_id": calibration["camera_id"],
            "roi": calibration["roi"],
            "capture": value,
            "artifact": artifact,
        }

    def finalize(self) -> list[str]:
        forced: list[str] = []
        if self.services:
            try:
                self.stop_services()
            except (OSError, RuntimeError, TimeoutError, ValueError) as error:
                self.run.event("live.bodylink.force-stop-failed", error=str(error))
                forced.append("bodylink-services")
        if self.fixture_used:
            try:
                fixture.release_all()
                self.fixture_actuated = False
            except (OSError, RuntimeError, TimeoutError, ValueError) as error:
                self.run.event("live.fixture.release-failed", error=str(error))
                forced.append("fixture-release-failed")
        if self.network_lease is not None:
            try:
                network.clear()
                self.network_lease = None
            except (OSError, RuntimeError, TimeoutError, ValueError) as error:
                self.run.event("live.network.clear-failed", error=str(error))
                forced.append("network-clear-failed")
        try:
            if self.monitor is not None:
                payload = self.monitor.stop()
                self.monitor = None
                self.run.event(
                    "live.serial.monitor.stopped",
                    device_bytes=len(payload),
                )
        finally:
            if self.device_access is not None:
                self.device_access.__exit__(None, None, None)
                self.device_access = None
        return forced

    def state(self) -> dict[str, Any]:
        return {
            "fixture_released": not self.fixture_actuated,
            "network_lease": self.network_lease,
            "background_services": sorted(self.services),
        }
