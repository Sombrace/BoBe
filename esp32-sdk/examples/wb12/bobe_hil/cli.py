import argparse
import asyncio
import json
import subprocess
import sys
from pathlib import Path

from websockets.exceptions import WebSocketException

from bobe_hil import __version__, device, fixture, network, observer
from bobe_hil.bodylink import load_private_config, serve_once
from bobe_hil.doctor import render_json
from bobe_hil.evidence import EvidenceRun, challenge, verify_run
from bobe_hil.fixture_protocol import (
    MAX_PRESS_MS,
    MAX_RESET_MS,
    MIN_PRESS_MS,
    MIN_RESET_MS,
)
from bobe_hil.image import PROFILES, create_manifest, write_json
from bobe_hil.media import (
    audio_challenge,
    audio_device_list,
    audio_metrics,
    image_metrics,
    play_audio,
    play_record_audio,
    record_audio,
    rgb565_to_png,
    write_metrics,
    write_wav,
)
from bobe_hil.network_profiles import NETWORK_PROFILES
from bobe_hil.paths import APP_OFFSET, REPOSITORY_ROOT
from bobe_hil.readiness import readiness_for
from bobe_hil.runner import run_scenario, write_junit
from bobe_hil.scenario import list_scenarios, load_named_scenario
from bobe_hil.ui import render_ui_evidence


def print_json(value: object) -> None:
    sys.stdout.write(json.dumps(value, indent=2, sort_keys=True) + "\n")


def scenario_summary(name: str) -> dict[str, object]:
    scenario = load_named_scenario(name)
    return {
        "name": scenario.name,
        "description": scenario.description,
        "claim": scenario.claim,
        "requires": list(scenario.requires),
        "readiness": readiness_for(scenario.requires),
        "steps": [
            {
                "id": step.step_id,
                "operation": step.operation,
                "parameters": step.parameters,
            }
            for step in scenario.steps
        ],
        "cleanup": [
            {
                "id": step.step_id,
                "operation": step.operation,
                "parameters": step.parameters,
            }
            for step in scenario.cleanup
        ],
    }


def run_bodylink_stub(scenario: str, timeout: int) -> dict[str, object]:
    run = EvidenceRun.create(f"bodylink-{scenario}", "protocol")
    try:
        config = load_private_config(scenario, str(run.manifest["nonce"]))
        run.event(
            "bodylink.stub.started",
            scenario=scenario,
            timeout_seconds=timeout,
        )
        asyncio.run(serve_once(config, run, timeout=float(timeout)))
        run.finish("passed")
        return {
            "run_id": run.manifest["run_id"],
            "directory": str(run.directory),
            "status": run.manifest["status"],
            "claim": "protocol",
            "reason": None,
            "exit_code": 0,
        }
    except (
        OSError,
        RuntimeError,
        TimeoutError,
        ValueError,
        WebSocketException,
    ) as error:
        run.finish("failed", str(error))
        return {
            "run_id": run.manifest["run_id"],
            "directory": str(run.directory),
            "status": run.manifest["status"],
            "claim": "protocol",
            "reason": str(error),
            "exit_code": 1,
        }
    finally:
        run.close()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="bobe-hil",
        description=(
            "Project-local hardware and validation tooling for the BoBe WB-12. "
            "Physical commands act on real hardware when it is connected."
        ),
    )
    parser.add_argument(
        "--version",
        action="version",
        version=f"bobe-hil {__version__}",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser(
        "doctor",
        help="Report host, device, fixture, network, camera, and audio readiness.",
    )

    scenario_parser = subparsers.add_parser(
        "scenario",
        help="Inspect or run typed scenario contracts from hil/scenarios.",
    )
    scenario_subparsers = scenario_parser.add_subparsers(
        dest="scenario_command",
        required=True,
    )
    scenario_subparsers.add_parser(
        "list",
        help="List scenario names, claims, and declared requirements.",
    )
    show_parser = scenario_subparsers.add_parser(
        "show",
        help="Show the parsed contract and current readiness for one scenario.",
    )
    show_parser.add_argument("name", help="Scenario name from hil/scenarios.")
    run_help = (
        "Run one scenario from hil/scenarios, print its JSON result even for "
        "failed or blocked outcomes, and retain evidence; scenarios that need "
        "absent hardware are blocked and skipped in JUnit."
    )
    run_parser = scenario_subparsers.add_parser(
        "run",
        help=run_help,
        description=run_help,
    )
    run_parser.add_argument("name", help="Scenario name from hil/scenarios.")
    execution = run_parser.add_mutually_exclusive_group(required=True)
    execution.add_argument(
        "--dry-run",
        action="store_true",
        help=(
            "Print and record the step plan without touching hardware; "
            "status is planned with exit 0."
        ),
    )
    execution.add_argument(
        "--live",
        action="store_true",
        help=(
            "Execute the scenario steps against real hardware after a "
            "hardware availability preflight."
        ),
    )
    execution.add_argument(
        "--mock",
        action="store_true",
        help=(
            "Execute deterministic simulated adapters with claim simulation, "
            "never physical."
        ),
    )
    run_parser.add_argument(
        "--junit",
        type=Path,
        help="Write a new JUnit report; blocked results are skipped.",
    )

    evidence_parser = subparsers.add_parser(
        "evidence",
        help="Create challenges or verify retained evidence integrity.",
    )
    evidence_subparsers = evidence_parser.add_subparsers(
        dest="evidence_command",
        required=True,
    )
    verify_parser = evidence_subparsers.add_parser(
        "verify",
        help="Verify manifest, event-chain, and artifact integrity.",
    )
    verify_parser.add_argument("directory", type=Path, help="Retained run directory.")
    challenge_parser = evidence_subparsers.add_parser(
        "challenge",
        help="Derive deterministic challenge bytes for a run nonce and channel.",
    )
    challenge_parser.add_argument("nonce", help="Run nonce.")
    challenge_parser.add_argument("channel", help="Challenge channel name.")

    bodylink_parser = subparsers.add_parser(
        "bodylink",
        help="Run a bounded protocol peer on the configured isolated interface.",
    )
    bodylink_subparsers = bodylink_parser.add_subparsers(
        dest="bodylink_command",
        required=True,
    )
    serve_parser = bodylink_subparsers.add_parser(
        "serve",
        help="Serve exactly one configured BodyLink protocol session.",
    )
    serve_parser.add_argument(
        "scenario",
        choices=("handshake", "voice-roundtrip"),
        help="Bounded protocol behavior to serve.",
    )
    serve_parser.add_argument(
        "--timeout",
        type=int,
        choices=range(30, 601),
        default=180,
        metavar="SECONDS",
        help="Wall-time bound (30-600 seconds; default: 180).",
    )

    media_parser = subparsers.add_parser(
        "media",
        help="Generate/analyze media files or perform bounded CoreAudio I/O.",
    )
    media_subparsers = media_parser.add_subparsers(
        dest="media_command",
        required=True,
    )
    generate_audio_parser = media_subparsers.add_parser(
        "generate-audio",
        help="Generate deterministic mono PCM16 from a 32-hex nonce.",
    )
    generate_audio_parser.add_argument("nonce", help="32 lowercase hexadecimal digits.")
    generate_audio_parser.add_argument("output", type=Path, help="New WAV path.")
    generate_audio_parser.add_argument(
        "--sample-rate",
        type=int,
        default=48000,
        help="Sample rate from 8000 to 96000 Hz (default: 48000).",
    )
    generate_audio_parser.add_argument(
        "--duration",
        type=float,
        default=3.0,
        help="Duration from 0.5 to 10 seconds (default: 3).",
    )
    analyze_audio_parser = media_subparsers.add_parser(
        "analyze-audio",
        help="Measure matching-rate mono PCM16 reference and capture files.",
    )
    analyze_audio_parser.add_argument("reference", type=Path, help="Reference WAV.")
    analyze_audio_parser.add_argument("capture", type=Path, help="Captured WAV.")
    analyze_audio_parser.add_argument(
        "--output",
        type=Path,
        help="New metrics JSON path; omit to print JSON.",
    )
    analyze_image_parser = media_subparsers.add_parser(
        "analyze-image",
        help="Measure decoded pixel differences for equal-size images.",
    )
    analyze_image_parser.add_argument("expected", type=Path, help="Expected image.")
    analyze_image_parser.add_argument("actual", type=Path, help="Actual image.")
    analyze_image_parser.add_argument(
        "--output",
        type=Path,
        help="New metrics JSON path; omit to print JSON.",
    )
    media_subparsers.add_parser(
        "audio-devices",
        help="List exact CoreAudio input/output names and capabilities.",
    )
    record_help = (
        "Record bounded mono PCM16 from one exact CoreAudio input; opens a live "
        "microphone, may prompt macOS TCC, and persists ambient audio."
    )
    record_audio_parser = media_subparsers.add_parser(
        "record-audio",
        help=record_help,
        description=record_help,
    )
    record_audio_parser.add_argument(
        "--device",
        required=True,
        help="Exact input name from audio-devices.",
    )
    record_audio_parser.add_argument(
        "--output",
        required=True,
        type=Path,
        help="New WAV path.",
    )
    record_audio_parser.add_argument(
        "--duration",
        type=float,
        default=3.0,
        help="Duration from 0.1 to 30 seconds (default: 3).",
    )
    record_audio_parser.add_argument(
        "--sample-rate",
        type=int,
        default=48000,
        help="Sample rate from 8000 to 96000 Hz (default: 48000).",
    )
    play_help = (
        "Play bounded mono PCM16 through one exact CoreAudio output; produces "
        "audible sound and the digital peak bound is not an SPL guarantee."
    )
    play_audio_parser = media_subparsers.add_parser(
        "play-audio",
        help=play_help,
        description=play_help,
    )
    play_audio_parser.add_argument(
        "--device",
        required=True,
        help="Exact output name from audio-devices.",
    )
    play_audio_parser.add_argument(
        "--input",
        required=True,
        type=Path,
        help="Mono PCM16 WAV, at most 10 seconds with bounded digital peak.",
    )
    play_record_help = (
        "Synchronize audible playback and live microphone capture on exact "
        "devices; may prompt TCC and persists ambient audio."
    )
    play_record_parser = media_subparsers.add_parser(
        "play-record",
        help=play_record_help,
        description=play_record_help,
    )
    play_record_parser.add_argument(
        "--input-device",
        required=True,
        help="Exact input name from audio-devices.",
    )
    play_record_parser.add_argument(
        "--output-device",
        required=True,
        help="Exact output name from audio-devices.",
    )
    play_record_parser.add_argument(
        "--stimulus",
        required=True,
        type=Path,
        help="Bounded mono PCM16 WAV to play.",
    )
    play_record_parser.add_argument(
        "--capture",
        required=True,
        type=Path,
        help="New captured WAV path.",
    )
    play_record_parser.add_argument(
        "--tail",
        type=float,
        default=1.0,
        help="Post-stimulus capture from 0 to 5 seconds (default: 1).",
    )
    convert_image_parser = media_subparsers.add_parser(
        "rgb565-to-png",
        help="Convert explicit-dimension big-endian RGB565 to PNG.",
    )
    convert_image_parser.add_argument("source", type=Path, help="RGB565 input.")
    convert_image_parser.add_argument("output", type=Path, help="New PNG path.")
    convert_image_parser.add_argument(
        "--width",
        type=int,
        required=True,
        help="Image width from 1 to 4096.",
    )
    convert_image_parser.add_argument(
        "--height",
        type=int,
        required=True,
        help="Image height from 1 to 4096.",
    )

    ui_parser = subparsers.add_parser(
        "ui",
        help="Retain deterministic logical LVGL render evidence.",
    )
    ui_subparsers = ui_parser.add_subparsers(
        dest="ui_command",
        required=True,
    )
    ui_subparsers.add_parser(
        "render",
        help="Compare reference pixels and retain named RGB565/PNG artifacts.",
    )

    artifact_parser = subparsers.add_parser(
        "artifact",
        help="Create unsigned build provenance records.",
    )
    artifact_subparsers = artifact_parser.add_subparsers(
        dest="artifact_command",
        required=True,
    )
    manifest_parser = artifact_subparsers.add_parser(
        "create-manifest",
        help="Create an adjacent unsigned firmware manifest.",
    )
    manifest_parser.add_argument(
        "build_directory",
        type=Path,
        help="Repository build directory containing bobe_wb12.bin/.elf.",
    )
    manifest_parser.add_argument(
        "--profile",
        required=True,
        choices=sorted(PROFILES),
        help="Firmware profile recorded in the manifest.",
    )

    device_parser = subparsers.add_parser(
        "device",
        help="Inspect, observe, or flash a WB-12 over USB.",
    )
    device_subparsers = device_parser.add_subparsers(
        dest="device_command",
        required=True,
    )
    device_subparsers.add_parser(
        "status",
        help="Report the WB-12 boards connected over USB.",
    )
    identify_parser = device_subparsers.add_parser(
        "identify",
        help="Report the MAC of the connected WB-12 or of an explicit port.",
        description=(
            "Without --port, read the MAC from the USB serial number of the "
            "one connected WB-12. With --port, reset that ESP32-S3 through "
            "the ROM loader and read its MAC."
        ),
    )
    identify_parser.add_argument(
        "--port",
        help="Explicit serial endpoint to reset and inspect.",
    )
    observe_parser = device_subparsers.add_parser(
        "observe",
        help="Watch device serial output for caller-selected markers.",
        description=(
            "Watch device serial output for caller-selected markers. Opens the "
            "device serial endpoint exclusively for the duration."
        ),
    )
    observe_parser.add_argument(
        "--seconds",
        type=float,
        default=device.DEFAULT_OBSERVE_SECONDS,
        help=f"Observation bound (default: {device.DEFAULT_OBSERVE_SECONDS}).",
    )
    observe_parser.add_argument(
        "--expect",
        action="append",
        default=None,
        metavar="TEXT",
        help="Required marker; repeatable (default: boot markers).",
    )
    observe_parser.add_argument(
        "--reject",
        action="append",
        default=None,
        metavar="TEXT",
        help="Failure marker; repeatable (default: crash markers).",
    )
    observe_parser.add_argument(
        "--port",
        help="Exact serial endpoint; omit to use the one connected WB-12.",
    )
    observe_parser.add_argument(
        "--identity",
        help=(
            "ESP MAC of the WB-12 to observe; omit to take it from the USB "
            "serial number."
        ),
    )
    observe_parser.add_argument(
        "--reset",
        action="store_true",
        help="Hard-reset through the ROM loader before observing.",
    )
    flash_help = (
        "Write and read-back verify one image, then observe the reboot. A "
        "manifest and selected OTA slot let the result bind the boot to that "
        "image."
    )
    flash_parser = device_subparsers.add_parser(
        "flash",
        help=flash_help,
        description=flash_help,
    )
    flash_parser.add_argument(
        "--image",
        type=Path,
        required=True,
        help="Application image to write.",
    )
    flash_parser.add_argument(
        "--manifest",
        type=Path,
        help="Manifest to bind the image to; defaults to IMAGE.manifest.json.",
    )
    flash_parser.add_argument(
        "--offset",
        default=hex(APP_OFFSET),
        help=f"4 KiB aligned flash offset (default: {hex(APP_OFFSET)}).",
    )
    flash_parser.add_argument(
        "--rollback-image",
        type=Path,
        help="Image written back if the primary write or boot fails.",
    )
    flash_parser.add_argument(
        "--rollback-manifest",
        type=Path,
        help="Manifest for the rollback image.",
    )
    flash_parser.add_argument(
        "--observe-seconds",
        type=float,
        default=device.DEFAULT_OBSERVE_SECONDS,
        help="Boot observation bound after the write.",
    )
    flash_parser.add_argument(
        "--port",
        help="Exact serial endpoint; omit to use the one connected WB-12.",
    )
    flash_parser.add_argument(
        "--no-fixture",
        action="store_true",
        help="Enter the ROM loader over USB only, ignoring the Pico fixture.",
    )

    fixture_parser = subparsers.add_parser(
        "fixture",
        help="Drive the Pico fixture contacts wired to the WB-12.",
    )
    fixture_subparsers = fixture_parser.add_subparsers(
        dest="fixture_command",
        required=True,
    )
    fixture_subparsers.add_parser(
        "status",
        help="Verify fixture identity, firmware, handshake, and outputs.",
    )
    press_parser = fixture_subparsers.add_parser(
        "press",
        help="Close one contact for a bounded duration, then release it.",
    )
    press_parser.add_argument(
        "--contact",
        required=True,
        choices=fixture.CONTACTS,
        help="Contact to actuate.",
    )
    press_parser.add_argument(
        "--ms",
        type=int,
        default=fixture.DEFAULT_PRESS_MS,
        metavar="MILLISECONDS",
        help=(
            f"Press duration {MIN_PRESS_MS}-{MAX_PRESS_MS} "
            f"(default: {fixture.DEFAULT_PRESS_MS})."
        ),
    )
    chord_parser = fixture_subparsers.add_parser(
        "chord",
        help="Apply one wired multi-contact chord for a bounded duration.",
    )
    chord_parser.add_argument(
        "--name",
        required=True,
        choices=fixture.CHORD_NAMES,
        help="Chord to apply.",
    )
    chord_parser.add_argument(
        "--ms",
        type=int,
        default=fixture.DEFAULT_CHORD_MS,
        metavar="MILLISECONDS",
        help=(
            f"Chord duration {MIN_PRESS_MS}-{MAX_PRESS_MS} "
            f"(default: {fixture.DEFAULT_CHORD_MS})."
        ),
    )
    reset_parser = fixture_subparsers.add_parser(
        "reset",
        help="Pulse WB-12 EN and wait for the fixture to release it.",
    )
    reset_parser.add_argument(
        "--ms",
        type=int,
        default=fixture.DEFAULT_RESET_MS,
        metavar="MILLISECONDS",
        help=(
            f"Reset pulse {MIN_RESET_MS}-{MAX_RESET_MS} "
            f"(default: {fixture.DEFAULT_RESET_MS})."
        ),
    )
    fixture_subparsers.add_parser(
        "enter-loader",
        help="Hold BOOT across an EN pulse so the WB-12 enters the ROM loader.",
    )
    fixture_subparsers.add_parser(
        "release-all",
        help="Release every fixture contact.",
    )

    net_parser = subparsers.add_parser(
        "net",
        help="Apply or clear bounded OpenWrt network impairment leases.",
    )
    net_subparsers = net_parser.add_subparsers(
        dest="net_command",
        required=True,
    )
    net_subparsers.add_parser(
        "status",
        help="Reconcile and report the local and router lease state.",
    )
    apply_parser = net_subparsers.add_parser(
        "apply",
        help="Apply one impairment profile under a router-expiring TTL.",
    )
    apply_parser.add_argument(
        "--profile",
        required=True,
        choices=sorted(NETWORK_PROFILES),
        help="Impairment profile; clean clears any active lease.",
    )
    apply_parser.add_argument(
        "--ttl",
        type=int,
        default=network.DEFAULT_TTL_SECONDS,
        metavar="SECONDS",
        help=(
            f"Router-side lease TTL "
            f"{network.MIN_TTL_SECONDS}-{network.MAX_TTL_SECONDS} "
            f"(default: {network.DEFAULT_TTL_SECONDS})."
        ),
    )
    net_subparsers.add_parser(
        "clear",
        help="Clear and verify the lease recorded by this repository.",
    )

    observer_parser = subparsers.add_parser(
        "observer",
        help="Inspect the independent Pi observer identity and reachability.",
    )
    observer_subparsers = observer_parser.add_subparsers(
        dest="observer_command",
        required=True,
    )
    observer_subparsers.add_parser(
        "status",
        help="Validate the pinned observer TLS identity without connecting.",
    )
    observer_subparsers.add_parser(
        "probe",
        help="Validate identity and complete one bounded mTLS status request.",
    )
    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    try:
        if args.command == "doctor":
            sys.stdout.write(render_json() + "\n")
        elif args.command == "scenario":
            if args.scenario_command == "list":
                print_json(
                    [
                        {
                            "name": scenario.name,
                            "description": scenario.description,
                            "claim": scenario.claim,
                            "requires": list(scenario.requires),
                        }
                        for scenario in list_scenarios()
                    ]
                )
            elif args.scenario_command == "show":
                print_json(scenario_summary(args.name))
            else:
                scenario = load_named_scenario(args.name)
                result = run_scenario(
                    scenario,
                    execution_mode=(
                        "dry-run"
                        if args.dry_run
                        else "mock"
                        if args.mock
                        else "live"
                    ),
                )
                if args.junit is not None:
                    write_junit(result, args.junit)
                    result["junit"] = str(args.junit)
                print_json(result)
                raise SystemExit(result["exit_code"])
        elif args.command == "bodylink":
            result = run_bodylink_stub(args.scenario, args.timeout)
            print_json(result)
            raise SystemExit(result["exit_code"])
        elif args.command == "media":
            if args.media_command == "audio-devices":
                print_json(audio_device_list())
            elif args.media_command == "generate-audio":
                write_wav(
                    args.output,
                    audio_challenge(
                        args.nonce,
                        args.sample_rate,
                        args.duration,
                    ),
                    args.sample_rate,
                )
                print_json(
                    {
                        "output": str(args.output),
                        "sample_rate": args.sample_rate,
                        "duration": args.duration,
                    }
                )
            elif args.media_command == "analyze-audio":
                write_metrics(
                    args.output,
                    audio_metrics(args.reference, args.capture),
                )
            elif args.media_command == "analyze-image":
                write_metrics(
                    args.output,
                    image_metrics(args.expected, args.actual),
                )
            elif args.media_command == "record-audio":
                print_json(
                    record_audio(
                        args.device,
                        args.output,
                        args.duration,
                        args.sample_rate,
                    )
                )
            elif args.media_command == "play-audio":
                print_json(play_audio(args.device, args.input))
            elif args.media_command == "play-record":
                print_json(
                    play_record_audio(
                        args.input_device,
                        args.output_device,
                        args.stimulus,
                        args.capture,
                        args.tail,
                    )
                )
            else:
                rgb565_to_png(
                    args.source,
                    args.output,
                    args.width,
                    args.height,
                )
                print_json(
                    {
                        "source": str(args.source),
                        "output": str(args.output),
                        "width": args.width,
                        "height": args.height,
                    }
                )
        elif args.command == "ui":
            print_json(render_ui_evidence())
        elif args.command == "artifact":
            build_directory = args.build_directory.resolve(strict=True)
            image = build_directory / "bobe_wb12.bin"
            elf = build_directory / "bobe_wb12.elf"
            output = Path(f"{image}.manifest.json")
            manifest = create_manifest(
                REPOSITORY_ROOT,
                image,
                elf,
                args.profile,
            )
            write_json(output, manifest)
            print_json(
                {
                    "manifest": str(output),
                    "image_sha256": manifest["image_sha256"],
                    "app_elf_sha256": manifest["app_elf_sha256"],
                    "build_number": manifest["build_number"],
                    "source_dirty": manifest["source_dirty"],
                }
            )
        elif args.command == "device":
            if args.device_command == "status":
                print_json(device.inspect_device())
            elif args.device_command == "identify":
                print_json(device.identify(port=args.port))
            elif args.device_command == "observe":
                observation = (
                    device.boot_observe if args.reset else device.observe
                )
                print_json(
                    observation(
                        seconds=args.seconds,
                        expect=tuple(args.expect or device.BOOT_REQUIRED),
                        reject=tuple(args.reject or device.BOOT_REJECTED),
                        port=args.port,
                        identity=args.identity,
                    )
                )
            else:
                print_json(
                    device.flash(
                        args.image,
                        manifest=args.manifest,
                        offset=int(str(args.offset), 0),
                        rollback_image=args.rollback_image,
                        rollback_manifest=args.rollback_manifest,
                        observe_seconds=args.observe_seconds,
                        port=args.port,
                        use_fixture=not args.no_fixture,
                    )
                )
        elif args.command == "fixture":
            if args.fixture_command == "status":
                print_json(fixture.status())
            elif args.fixture_command == "press":
                print_json(fixture.press(args.contact, args.ms))
            elif args.fixture_command == "chord":
                print_json(fixture.chord(args.name, args.ms))
            elif args.fixture_command == "reset":
                print_json(fixture.reset(args.ms))
            elif args.fixture_command == "enter-loader":
                print_json(fixture.enter_loader())
            else:
                print_json(fixture.release_all())
        elif args.command == "net":
            if args.net_command == "status":
                print_json(network.status())
            elif args.net_command == "apply":
                print_json(network.apply(args.profile, args.ttl))
            else:
                print_json(network.clear())
        elif args.command == "observer":
            print_json(
                observer.inspect_observer(
                    live_probe=args.observer_command == "probe",
                )
            )
        elif args.evidence_command == "verify":
            print_json(verify_run(args.directory))
        else:
            print(challenge(args.nonce, args.channel))
    except (
        json.JSONDecodeError,
        OSError,
        RuntimeError,
        subprocess.CalledProcessError,
        ValueError,
    ) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
