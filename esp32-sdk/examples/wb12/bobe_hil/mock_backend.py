from typing import Any

import numpy as np
from PIL import Image

from bobe_hil.evidence import EvidenceRun
from bobe_hil.media import (
    audio_challenge,
    audio_metrics,
    sha256_file,
    write_metrics,
    write_wav,
)
from bobe_hil.scenario import ScenarioStep
from bobe_hil.ui import render_ui_step


class MockScenarioBackend:
    def __init__(self, run: EvidenceRun) -> None:
        self.run = run
        self.working = run.directory / "mock-working"
        self.working.mkdir(mode=0o700)
        self.fixture_released = True
        self.network_profile = "clean"
        self.background_services: set[str] = set()

    def execute(self, step: ScenarioStep) -> dict[str, Any]:
        handlers = {
            "bodylink_stop": self.bodylink_stop,
            "bodylink_stub": self.bodylink_stub,
            "capture_audio": self.capture_audio,
            "capture_frame": self.capture_frame,
            "capture_serial": self.capture_serial,
            "fixture_chord": self.fixture_chord,
            "fixture_press": self.fixture_press,
            "fixture_release_all": self.fixture_release_all,
            "fixture_reset": self.fixture_reset,
            "network_profile": self.apply_network_profile,
            "ui_golden": self.ui_golden,
        }
        handler = handlers.get(step.operation)
        if handler is None:
            raise RuntimeError(
                f"mock operation is not implemented: {step.operation}"
            )
        return handler(step)

    def ui_golden(self, _step: ScenarioStep) -> dict[str, Any]:
        return render_ui_step(self.run)

    def fixture_reset(self, step: ScenarioStep) -> dict[str, Any]:
        self.fixture_released = True
        self.run.event(
            "mock.fixture.reset",
            step_id=step.step_id,
            challenge=self.run.manifest["challenges"]["fixture"],
            settle_ms=200,
            pulse_ms=120,
            lockout_ms=500,
        )
        return {"fixture_released": True, "reset_completed": True}

    def fixture_press(self, step: ScenarioStep) -> dict[str, Any]:
        return self.fixture_actuation(step, "control")

    def fixture_chord(self, step: ScenarioStep) -> dict[str, Any]:
        return self.fixture_actuation(step, "chord")

    def fixture_actuation(
        self,
        step: ScenarioStep,
        field: str,
    ) -> dict[str, Any]:
        if not self.fixture_released:
            raise RuntimeError("mock fixture interlock is active")
        self.fixture_released = False
        self.run.event(
            "mock.fixture.actuated",
            step_id=step.step_id,
            challenge=self.run.manifest["challenges"]["fixture"],
            operation=step.operation,
            target=step.parameters[field],
            duration_ms=step.parameters["duration_ms"],
        )
        self.fixture_released = True
        return {
            "target": step.parameters[field],
            "duration_ms": step.parameters["duration_ms"],
            "released": True,
        }

    def fixture_release_all(self, step: ScenarioStep) -> dict[str, Any]:
        self.fixture_released = True
        self.run.event(
            "mock.fixture.released",
            step_id=step.step_id,
        )
        return {"released": True}

    def apply_network_profile(self, step: ScenarioStep) -> dict[str, Any]:
        profile = str(step.parameters["profile"])
        self.network_profile = profile
        self.run.event(
            "mock.network.profile",
            step_id=step.step_id,
            challenge=self.run.manifest["challenges"]["network"],
            profile=profile,
            ttl_seconds=step.parameters["ttl_seconds"],
        )
        return {"profile": profile, "verified": True}

    def capture_serial(self, step: ScenarioStep) -> dict[str, Any]:
        lines = [
            "mock serial session",
            *[str(value) for value in step.parameters["expect"]],
            "health uptime=10s",
        ]
        rendered = "\n".join(lines) + "\n"
        for rejected in step.parameters["reject"]:
            if str(rejected) in rendered:
                raise RuntimeError("mock serial fixture contains rejected text")
        path = self.working / f"{step.step_id}.log"
        path.write_text(rendered, encoding="utf-8")
        path.chmod(0o600)
        artifact = self.run.add_artifact(
            path,
            "mock-serial",
            f"{step.step_id}.log",
        )
        self.run.event(
            "mock.serial.captured",
            step_id=step.step_id,
            expected=step.parameters["expect"],
            rejected=step.parameters["reject"],
            artifact_sha256=artifact["sha256"],
        )
        return {
            "expected_found": list(step.parameters["expect"]),
            "rejected_found": [],
            "artifact": artifact,
        }

    def bodylink_stub(self, step: ScenarioStep) -> dict[str, Any]:
        service_id = (
            "mock-bodylink-"
            + self.run.manifest["challenges"]["bodylink"][:16]
        )
        if bool(step.parameters["background"]):
            self.background_services.add(service_id)
        self.run.event(
            "mock.bodylink.started",
            step_id=step.step_id,
            service_id=service_id,
            scenario=step.parameters["scenario"],
            background=step.parameters["background"],
        )
        return {
            "service_id": service_id,
            "background": step.parameters["background"],
            "ready": True,
        }

    def bodylink_stop(self, step: ScenarioStep) -> dict[str, Any]:
        stopped = sorted(self.background_services)
        self.background_services.clear()
        self.run.event(
            "mock.bodylink.stopped",
            step_id=step.step_id,
            service_ids=stopped,
        )
        return {"stopped": stopped}

    def capture_audio(self, step: ScenarioStep) -> dict[str, Any]:
        duration = float(step.parameters["duration_seconds"])
        nonce = str(self.run.manifest["nonce"])
        reference = audio_challenge(nonce, 48000, duration)
        delay_samples = 960
        capture = np.concatenate(
            (
                np.zeros(delay_samples, dtype=np.int16),
                reference,
            )
        )
        reference_path = self.working / f"{step.step_id}-reference.wav"
        capture_path = self.working / f"{step.step_id}-capture.wav"
        metrics_path = self.working / f"{step.step_id}-metrics.json"
        write_wav(reference_path, reference, 48000)
        write_wav(capture_path, capture, 48000)
        metrics = audio_metrics(reference_path, capture_path)
        write_metrics(metrics_path, metrics)
        artifacts = [
            self.run.add_artifact(
                reference_path,
                "mock-audio-reference",
            ),
            self.run.add_artifact(
                capture_path,
                "mock-audio-capture",
            ),
            self.run.add_artifact(
                metrics_path,
                "mock-audio-metrics",
            ),
        ]
        self.run.event(
            "mock.audio.captured",
            step_id=step.step_id,
            direction=step.parameters["direction"],
            latency_samples=metrics["latency_samples"],
            correlation=metrics["normalized_correlation"],
        )
        return {
            "direction": step.parameters["direction"],
            "metrics": metrics,
            "artifacts": artifacts,
        }

    def capture_frame(self, step: ScenarioStep) -> dict[str, Any]:
        challenge = self.run.manifest["challenges"]["visual"]
        color = tuple(
            int(challenge[index : index + 2], 16)
            for index in (0, 2, 4)
        )
        image = Image.new("RGB", (280, 240), color)
        for index in range(16):
            value = int(challenge[index * 2 : index * 2 + 2], 16)
            image.putpixel((index, 0), (value, value ^ 0x55, value ^ 0xAA))
        path = self.working / f"{step.step_id}.png"
        image.save(path, format="PNG", optimize=False)
        path.chmod(0o600)
        artifact = self.run.add_artifact(
            path,
            "mock-frame",
            f"{step.step_id}.png",
        )
        self.run.event(
            "mock.frame.captured",
            step_id=step.step_id,
            state=step.parameters["state"],
            artifact_sha256=artifact["sha256"],
        )
        return {
            "state": step.parameters["state"],
            "width": 280,
            "height": 240,
            "sha256": sha256_file(path),
            "artifact": artifact,
        }

    def finalize(self) -> list[str]:
        forced: list[str] = []
        if self.background_services:
            stopped = sorted(self.background_services)
            self.background_services.clear()
            forced.append("bodylink-services")
            self.run.event(
                "mock.bodylink.force-stopped",
                service_ids=stopped,
            )
        if not self.fixture_released:
            self.fixture_released = True
            forced.append("fixture-release")
            self.run.event("mock.fixture.force-released")
        if self.network_profile != "clean":
            previous = self.network_profile
            self.network_profile = "clean"
            forced.append("network-profile")
            self.run.event(
                "mock.network.force-cleared",
                previous_profile=previous,
            )
        return forced

    def state(self) -> dict[str, Any]:
        return {
            "fixture_released": self.fixture_released,
            "network_profile": self.network_profile,
            "background_services": sorted(self.background_services),
        }
