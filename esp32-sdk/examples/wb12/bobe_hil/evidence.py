import hashlib
import json
import os
import secrets
import shutil
import threading
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from bobe_hil.paths import REPOSITORY_ROOT, RUNS_ROOT, git_metadata, sha256_file


EVIDENCE_SCHEMA = 2
MAX_EVIDENCE_ARTIFACT_BYTES = 128 * 1024 * 1024
ZERO_HASH = "0" * 64
CLAIMS = {"logical", "protocol", "physical", "simulation"}
TARGET_CLAIMS = {"logical", "protocol", "physical"}


def utc_now() -> str:
    return datetime.now(UTC).isoformat(timespec="milliseconds")


def valid_sha256(value: Any) -> bool:
    if not isinstance(value, str) or len(value) != 64:
        return False
    try:
        int(value, 16)
    except ValueError:
        return False
    return value == value.lower()


def source_state() -> dict[str, Any]:
    commit, dirty = git_metadata(REPOSITORY_ROOT)
    return {"commit": commit, "dirty": dirty}


def challenge(nonce: str, channel: str) -> str:
    if len(nonce) != 32 or any(character not in "0123456789abcdef" for character in nonce):
        raise ValueError("run nonce must be 16 lowercase hexadecimal bytes")
    if not channel or len(channel) > 64 or not channel.replace("-", "").isalnum():
        raise ValueError("challenge channel is malformed")
    material = f"bobe-hil-v1:{nonce}:{channel}".encode()
    return hashlib.sha256(material).hexdigest()


def exclusive_directory(path: Path) -> None:
    if path.is_symlink():
        raise ValueError("run directory must not be a symbolic link")
    path.mkdir(mode=0o700, parents=False, exist_ok=False)


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    if path.is_symlink():
        raise ValueError("manifest must not be a symbolic link")
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
    descriptor = os.open(
        temporary,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL,
        0o600,
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


@dataclass
class EvidenceRun:
    directory: Path
    manifest: dict[str, Any]
    _event_descriptor: int = field(repr=False)
    _event_hash: str = field(default=ZERO_HASH, repr=False)
    _mutex: threading.RLock = field(
        default_factory=threading.RLock,
        repr=False,
    )

    @classmethod
    def create(
        cls,
        scenario: str,
        claim: str = "logical",
        target_claim: str | None = None,
        scenario_contract_sha256: str | None = None,
    ) -> "EvidenceRun":
        if not scenario or len(scenario) > 64:
            raise ValueError("scenario name is malformed")
        if claim not in CLAIMS:
            raise ValueError("evidence claim is invalid")
        if claim == "simulation":
            if target_claim not in TARGET_CLAIMS:
                raise ValueError("simulation target claim is invalid")
        elif target_claim is not None:
            raise ValueError("target claim is accepted only for simulation")
        if (
            scenario_contract_sha256 is not None
            and not valid_sha256(scenario_contract_sha256)
        ):
            raise ValueError("scenario contract digest is malformed")
        RUNS_ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
        if RUNS_ROOT.is_symlink():
            raise ValueError("HIL runs root must not be a symbolic link")
        nonce = secrets.token_hex(16)
        run_id = (
            datetime.now(UTC).strftime("%Y%m%dT%H%M%S.%fZ")
            + "-"
            + nonce[:8]
        )
        directory = RUNS_ROOT / run_id
        exclusive_directory(directory)
        artifacts = directory / "artifacts"
        artifacts.mkdir(mode=0o700)
        event_path = directory / "events.jsonl"
        event_descriptor = os.open(
            event_path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_APPEND,
            0o600,
        )
        manifest: dict[str, Any] = {
            "schema_version": EVIDENCE_SCHEMA,
            "run_id": run_id,
            "scenario": scenario,
            "nonce": nonce,
            "created_at": utc_now(),
            "status": "running",
            "claim": claim,
            "event_chain_sha256": ZERO_HASH,
            "source": source_state(),
            "challenges": {
                channel: challenge(nonce, channel)
                for channel in ("audio", "bodylink", "fixture", "network", "visual")
            },
            "artifacts": [],
        }
        if target_claim is not None:
            manifest["target_claim"] = target_claim
        if scenario_contract_sha256 is not None:
            manifest["scenario_contract_sha256"] = (
                scenario_contract_sha256
            )
        atomic_json(directory / "manifest.json", manifest)
        return cls(directory, manifest, event_descriptor)

    def event(self, kind: str, **fields: Any) -> None:
        with self._mutex:
            if (
                self.manifest.get("status") != "running"
                or self._event_descriptor < 0
            ):
                raise RuntimeError("cannot append to a finished evidence run")
            if not kind or len(kind) > 80:
                raise ValueError("event kind is malformed")
            record = {
                "schema_version": EVIDENCE_SCHEMA,
                "timestamp": utc_now(),
                "kind": kind,
                "previous_sha256": self._event_hash,
                **fields,
            }
            digest = hashlib.sha256(
                json.dumps(
                    record,
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode()
            ).hexdigest()
            record["record_sha256"] = digest
            rendered = (
                json.dumps(record, separators=(",", ":"), sort_keys=True)
                + "\n"
            ).encode()
            os.write(self._event_descriptor, rendered)
            os.fsync(self._event_descriptor)
            self._event_hash = digest
            self.manifest["event_chain_sha256"] = digest
            atomic_json(self.directory / "manifest.json", self.manifest)

    def add_artifact(
        self,
        source: Path,
        kind: str,
        name: str | None = None,
    ) -> dict[str, Any]:
        with self._mutex:
            if self.manifest.get("status") != "running":
                raise RuntimeError(
                    "cannot add artifacts to a finished evidence run"
                )
            if source.is_symlink() or not source.is_file():
                raise ValueError(
                    "artifact source must be a regular non-symlink file"
                )
            logical_name = name or source.name
            if (
                not logical_name
                or len(logical_name) > 160
                or "/" in logical_name
                or "\\" in logical_name
            ):
                raise ValueError("artifact logical name is malformed")
            size = source.stat().st_size
            if size > MAX_EVIDENCE_ARTIFACT_BYTES:
                raise ValueError("artifact exceeds the 128 MiB evidence limit")
            digest = sha256_file(source)
            suffix = source.suffix.lower()
            destination = self.directory / "artifacts" / f"{digest}{suffix}"
            if not destination.exists():
                descriptor = os.open(
                    destination,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                    0o600,
                )
                with (
                    os.fdopen(descriptor, "wb") as output,
                    source.open("rb") as input_file,
                ):
                    shutil.copyfileobj(input_file, output)
            artifact = {
                "kind": kind,
                "name": logical_name,
                "sha256": digest,
                "size": size,
                "path": str(destination.relative_to(self.directory)),
            }
            self.manifest["artifacts"].append(artifact)
            atomic_json(self.directory / "manifest.json", self.manifest)
            return artifact

    def finish(self, status: str, reason: str | None = None) -> None:
        with self._mutex:
            if status not in {"blocked", "failed", "passed", "planned"}:
                raise ValueError("invalid terminal evidence status")
            self.event("run.finished", status=status, reason=reason)
            self.manifest["status"] = status
            self.manifest["finished_at"] = utc_now()
            if reason is not None:
                self.manifest["reason"] = reason
            atomic_json(self.directory / "manifest.json", self.manifest)
            os.close(self._event_descriptor)
            self._event_descriptor = -1

    def close(self) -> None:
        with self._mutex:
            if self._event_descriptor >= 0:
                if not self.directory.is_dir():
                    os.close(self._event_descriptor)
                    self._event_descriptor = -1
                    return
                if self.manifest.get("status") == "running":
                    self.finish("failed", "run-closed-without-terminal-status")
                else:
                    os.close(self._event_descriptor)
                    self._event_descriptor = -1


def verify_run(directory: Path) -> dict[str, Any]:
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("evidence run must be a regular directory")
    manifest_path = directory / "manifest.json"
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ValueError("evidence manifest is missing or unsafe")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    schema_version = manifest.get("schema_version")
    if schema_version != EVIDENCE_SCHEMA:
        raise ValueError("unsupported evidence schema")
    claim = manifest.get("claim")
    if claim not in CLAIMS:
        raise ValueError("unsupported evidence claim")
    target_claim = manifest.get("target_claim")
    if (
        claim == "simulation"
        and target_claim not in TARGET_CLAIMS
    ) or (claim != "simulation" and target_claim is not None):
        raise ValueError("unsupported evidence target claim")
    nonce = manifest.get("nonce")
    expected_challenges = {
        channel: challenge(nonce, channel)
        for channel in ("audio", "bodylink", "fixture", "network", "visual")
    }
    if manifest.get("challenges") != expected_challenges:
        raise ValueError("evidence challenges do not match run nonce")
    events_path = directory / "events.jsonl"
    if events_path.is_symlink() or not events_path.is_file():
        raise ValueError("evidence event log is missing or unsafe")
    events: list[dict[str, Any]] = []
    previous_hash = ZERO_HASH
    for line_number, line in enumerate(
        events_path.read_text(encoding="utf-8").splitlines(),
        start=1,
    ):
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(
                f"evidence event line {line_number} is malformed"
            ) from error
        if (
            not isinstance(event, dict)
            or event.get("schema_version") != schema_version
            or not isinstance(event.get("kind"), str)
            or not isinstance(event.get("timestamp"), str)
        ):
            raise ValueError(
                f"evidence event line {line_number} has invalid fields"
            )
        if (
            event.get("previous_sha256") != previous_hash
            or not isinstance(event.get("record_sha256"), str)
        ):
            raise ValueError("evidence event chain fields are invalid")
        stored_hash = event["record_sha256"]
        unsigned = {
            key: value
            for key, value in event.items()
            if key != "record_sha256"
        }
        calculated_hash = hashlib.sha256(
            json.dumps(
                unsigned,
                separators=(",", ":"),
                sort_keys=True,
            ).encode()
        ).hexdigest()
        if stored_hash != calculated_hash:
            raise ValueError("evidence event chain hash is invalid")
        previous_hash = stored_hash
        events.append(event)
    if not events or events[-1].get("kind") != "run.finished":
        raise ValueError("evidence run has no terminal event")
    if events[-1].get("status") != manifest.get("status"):
        raise ValueError("evidence terminal status does not match manifest")
    if manifest.get("event_chain_sha256") != previous_hash:
        raise ValueError("evidence manifest event chain is stale")
    for artifact in manifest.get("artifacts", []):
        path = directory / artifact["path"]
        if path.is_symlink() or not path.is_file():
            raise ValueError("evidence artifact is missing or unsafe")
        if path.stat().st_size != artifact["size"]:
            raise ValueError("evidence artifact size mismatch")
        if sha256_file(path) != artifact["sha256"]:
            raise ValueError("evidence artifact digest mismatch")
    if manifest.get("status") == "passed":
        if claim == "physical":
            completed = [
                event for event in events if event.get("kind") == "step.completed"
            ]
            finalized = [
                event
                for event in events
                if event.get("kind") == "cleanup.finalized"
            ]
            if not completed:
                raise ValueError(
                    "physical pass lacks completed hardware step evidence"
                )
            if len(finalized) != 1:
                raise ValueError(
                    "physical pass lacks a single cleanup finalization"
                )
            if finalized[0].get("forced_cleanup"):
                raise ValueError(
                    "physical pass required forced cleanup of physical state"
                )
        kinds = {event["kind"] for event in events}
        scenario = manifest.get("scenario")
        artifacts = manifest.get("artifacts", [])
        if scenario == "ui-render":
            artifact_kinds = {artifact.get("kind") for artifact in artifacts}
            if not {"ui-png", "ui-rgb565"}.issubset(artifact_kinds):
                raise ValueError("passed UI evidence lacks visual artifacts")
            if "ui.render.completed" not in kinds:
                raise ValueError("passed UI evidence lacks completion event")
        elif isinstance(scenario, str) and scenario.startswith("bodylink-"):
            if "bodylink.control" not in kinds:
                raise ValueError("passed BodyLink evidence lacks protocol events")
        elif manifest.get("claim") == "physical":
            if "cleanup.finalized" not in kinds:
                raise ValueError("passed physical evidence lacks cleanup evidence")
        elif not artifacts or len(events) < 2:
            raise ValueError(
                "passed evidence lacks scenario artifacts or events"
            )
    return {
        "run_id": manifest["run_id"],
        "scenario": manifest["scenario"],
        "status": manifest["status"],
        "artifact_count": len(manifest.get("artifacts", [])),
        "event_count": len(events),
        "claim": claim,
        "target_claim": manifest.get("target_claim"),
        "chain_intact": True,
        "authenticated": False,
    }
