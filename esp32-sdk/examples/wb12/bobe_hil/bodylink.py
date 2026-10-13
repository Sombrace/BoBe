import asyncio
import hashlib
import json
import os
import ssl
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from websockets.asyncio.server import ServerConnection, serve
from websockets.typing import Subprotocol

from bobe_hil.evidence import EvidenceRun
from bobe_hil.paths import PRIVATE_ROOT


PROTOCOL_MAJOR = 1
PROTOCOL_MINOR = 1
SUBPROTOCOL = "bobe.body.v1"
MEDIA_HEADER = struct.Struct(">BBHIIQ")
MEDIA_MICROPHONE_PCM = 0x01
MEDIA_SPEAKER_PCM = 0x02
MEDIA_OPUS = 0x03
MEDIA_DIAGNOSTIC = 0x04
MEDIA_FLAG_DISCONTINUITY = 0x01
MICROPHONE_PAYLOAD_BYTES = 320 * 2
SPEAKER_SAMPLES_PER_FRAME = 480
SPEAKER_PAYLOAD_BYTES = SPEAKER_SAMPLES_PER_FRAME * 2
SPEAKER_SAMPLE_RATE = 24000
FRAME_DURATION_MS = 20
MAX_CONTROL_BYTES = 4096
MAX_PLAYBACK_CREDIT_MS = 300
MAX_MICROPHONE_FRAMES = 500
BODYLINK_PRIVATE_ROOT = PRIVATE_ROOT / "bodylink-pki"
CLAIM_PATH = BODYLINK_PRIVATE_ROOT / "claim.json"
SERVER_CERT_PATH = BODYLINK_PRIVATE_ROOT / "server-cert.pem"
SERVER_KEY_PATH = BODYLINK_PRIVATE_ROOT / "server-key.pem"
CA_CERT_PATH = BODYLINK_PRIVATE_ROOT / "ca-cert.pem"


class WebSocketLike(Protocol):
    async def recv(self) -> str | bytes: ...

    async def send(self, message: str | bytes) -> None: ...


@dataclass(frozen=True)
class BodyLinkConfig:
    expected_device_id: str
    controller_epoch: int
    scenario: str
    nonce: str


def fixed_private_file(path: Path, description: str) -> Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{description} is missing or unsafe")
    return path


def load_private_config(scenario: str, nonce: str) -> BodyLinkConfig:
    fixed_private_file(CLAIM_PATH, "BodyLink claim")
    try:
        claim = json.loads(CLAIM_PATH.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("BodyLink claim is not valid JSON") from error
    device_id = claim.get("device_id")
    controller_epoch = claim.get("controller_epoch")
    if not isinstance(device_id, str) or not device_id:
        raise ValueError("BodyLink claim device identity is missing")
    if type(controller_epoch) is not int or not 0 < controller_epoch <= 2**64 - 1:
        raise ValueError("BodyLink claim controller epoch is invalid")
    if scenario not in {"handshake", "voice-roundtrip"}:
        raise ValueError("unsupported BodyLink scenario")
    return BodyLinkConfig(device_id, controller_epoch, scenario, nonce)


def control_message(message_type: str, **fields: Any) -> str:
    return json.dumps(
        {
            "type": message_type,
            "protocol_major": PROTOCOL_MAJOR,
            **fields,
        },
        separators=(",", ":"),
        sort_keys=True,
    )


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("BodyLink control message contains a duplicate key")
        value[key] = item
    return value


def _reject_constant(value: str) -> Any:
    raise ValueError(f"BodyLink control message contains invalid JSON number: {value}")


def parse_control(message: str | bytes) -> dict[str, Any]:
    if not isinstance(message, str):
        raise ValueError("expected a BodyLink text control message")
    encoded = message.encode("utf-8")
    if len(encoded) > MAX_CONTROL_BYTES:
        raise ValueError("BodyLink control message exceeds 4096 bytes")
    try:
        value = json.loads(message, object_pairs_hook=_unique_object, parse_constant=_reject_constant)
    except json.JSONDecodeError as error:
        raise ValueError("BodyLink control message is malformed") from error
    if not isinstance(value, dict):
        raise ValueError("BodyLink control message must be an object")
    if (
        type(value.get("protocol_major")) is not int
        or value["protocol_major"] != PROTOCOL_MAJOR
    ):
        raise ValueError("BodyLink protocol major does not match")
    if not isinstance(value.get("type"), str):
        raise ValueError("BodyLink control type is missing")
    return value


def parse_media(message: str | bytes) -> tuple[dict[str, int], bytes]:
    if not isinstance(message, bytes) or len(message) < MEDIA_HEADER.size:
        raise ValueError("expected a complete BodyLink binary media frame")
    major, kind, flags, stream_id, sequence, timestamp_us = MEDIA_HEADER.unpack_from(
        message
    )
    if (major != PROTOCOL_MAJOR or stream_id == 0
            or flags & ~MEDIA_FLAG_DISCONTINUITY
            or kind not in {MEDIA_MICROPHONE_PCM, MEDIA_SPEAKER_PCM, MEDIA_OPUS, MEDIA_DIAGNOSTIC}):
        raise ValueError("BodyLink media header is invalid")
    payload = message[MEDIA_HEADER.size:]
    valid_size = {
        MEDIA_MICROPHONE_PCM: len(payload) == MICROPHONE_PAYLOAD_BYTES,
        MEDIA_SPEAKER_PCM: len(payload) == SPEAKER_PAYLOAD_BYTES,
        MEDIA_OPUS: 0 < len(payload) <= 512,
        MEDIA_DIAGNOSTIC: len(payload) <= 1024,
    }[kind]
    if not valid_size:
        raise ValueError("BodyLink media payload size is invalid")
    return (
        {
            "protocol_major": major,
            "kind": kind,
            "flags": flags,
            "stream_id": stream_id,
            "sequence": sequence,
            "timestamp_us": timestamp_us,
        },
        payload,
    )


def challenge_ids(nonce: str) -> dict[str, str | int]:
    digest = hashlib.sha256(f"bodylink:{nonce}".encode()).hexdigest()
    return {
        "session_id": f"hil-session-{digest[:16]}",
        "lease_id": f"hil-lease-{digest[16:32]}",
        "turn_id": f"hil-turn-{digest[32:48]}",
        "ping_nonce": int(digest[48:61], 16),
    }


def speaker_pcm(nonce: str, frame_count: int = 150) -> list[bytes]:
    if frame_count <= 0 or frame_count > 500:
        raise ValueError("speaker frame count is outside the bounded range")
    state = int(hashlib.sha256(f"audio:{nonce}".encode()).hexdigest()[:8], 16)
    frames: list[bytes] = []
    for _ in range(frame_count):
        samples = bytearray()
        for _ in range(SPEAKER_SAMPLES_PER_FRAME):
            state ^= (state << 13) & 0xFFFFFFFF
            state ^= state >> 17
            state ^= (state << 5) & 0xFFFFFFFF
            sample = ((state & 0xFFFF) - 32768) // 5
            samples.extend(struct.pack("<h", sample))
        frames.append(bytes(samples))
    return frames


class BodyLinkSession:
    def __init__(
        self,
        websocket: WebSocketLike,
        config: BodyLinkConfig,
        evidence: EvidenceRun,
    ) -> None:
        self.websocket = websocket
        self.config = config
        self.evidence = evidence
        self.ids = challenge_ids(config.nonce)
        self.hello: dict[str, Any] | None = None

    def retain_pcm(
        self,
        name: str,
        payload: bytes,
        kind: str,
    ) -> dict[str, Any]:
        if (
            name not in {
                "bodylink-microphone.pcm",
                "bodylink-speaker.pcm",
            }
            or not payload
        ):
            raise ValueError("BodyLink PCM artifact is invalid")
        temporary = self.evidence.directory / f".{name}.tmp"
        descriptor = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
        )
        try:
            try:
                offset = 0
                while offset < len(payload):
                    written = os.write(descriptor, payload[offset:])
                    if written <= 0:
                        raise OSError(
                            "BodyLink PCM artifact write was incomplete"
                        )
                    offset += written
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            return self.evidence.add_artifact(temporary, kind, name)
        finally:
            temporary.unlink(missing_ok=True)

    async def send_control(self, message_type: str, **fields: Any) -> None:
        rendered = control_message(message_type, **fields)
        await self.websocket.send(rendered)
        self.evidence.event(
            "bodylink.control",
            direction="to-device",
            message_type=message_type,
            sha256=hashlib.sha256(rendered.encode()).hexdigest(),
        )

    async def receive(self, timeout: float = 15.0) -> str | bytes:
        message = await asyncio.wait_for(self.websocket.recv(), timeout=timeout)
        if isinstance(message, str):
            control = parse_control(message)
            self.evidence.event(
                "bodylink.control",
                direction="from-device",
                message_type=control["type"],
                sha256=hashlib.sha256(message.encode()).hexdigest(),
            )
        else:
            header, payload = parse_media(message)
            self.evidence.event(
                "bodylink.media",
                direction="from-device",
                header=header,
                payload_size=len(payload),
                payload_sha256=hashlib.sha256(payload).hexdigest(),
            )
        return message

    async def receive_control(
        self,
        expected_type: str,
        timeout: float = 15.0,
    ) -> dict[str, Any]:
        deadline = asyncio.get_running_loop().time() + timeout
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise TimeoutError(f"timed out waiting for {expected_type}")
            message = await self.receive(remaining)
            if isinstance(message, bytes):
                raise ValueError(
                    f"unexpected BodyLink media while waiting for {expected_type}"
                )
            control = parse_control(message)
            if control["type"] == expected_type:
                return control

    async def handshake(self) -> None:
        hello = await self.receive_control("hello")
        if (
            type(hello.get("protocol_minor")) is not int
            or hello["protocol_minor"] != PROTOCOL_MINOR
            or hello.get("device_id") != self.config.expected_device_id
            or not isinstance(hello.get("boot_id"), str)
            or len(hello["boot_id"]) != 32
        ):
            raise ValueError("BodyLink hello identity or version is invalid")
        self.hello = hello
        await self.send_control(
            "welcome",
            protocol_minor=PROTOCOL_MINOR,
            session_id=self.ids["session_id"],
            connection_generation=1,
            heartbeat_interval_ms=10000,
            controller_epoch=self.config.controller_epoch,
        )

    async def ping(self) -> None:
        await self.send_control("ping", nonce=self.ids["ping_nonce"])
        pong = await self.receive_control("pong")
        if pong.get("nonce") != self.ids["ping_nonce"]:
            raise ValueError("BodyLink pong nonce does not match")

    async def voice_roundtrip(self) -> None:
        request = await self.receive_control("capture.request", timeout=60.0)
        request_id = request.get("request_id")
        capture_stream_id = request.get("stream_id")
        if (
            not isinstance(request_id, str)
            or not request_id
            or type(capture_stream_id) is not int
            or capture_stream_id <= 0
            or request.get("codec") != "pcm_s16le"
            or type(request.get("sample_rate_hz")) is not int
            or request["sample_rate_hz"] != 16000
            or type(request.get("channels")) is not int
            or request["channels"] != 1
            or type(request.get("frame_duration_ms")) is not int
            or request["frame_duration_ms"] != FRAME_DURATION_MS
            or request.get("trigger") not in {"action_button", "wake_word"}
        ):
            raise ValueError("capture request route is malformed")
        await self.send_control(
            "capture.granted",
            request_id=request_id,
            stream_id=capture_stream_id,
            lease_id=self.ids["lease_id"],
            turn_id=self.ids["turn_id"],
            lease_ttl_ms=120000,
        )
        capture_open = await self.receive_control("audio.capture.open")
        if (
            type(capture_open.get("stream_id")) is not int
            or capture_open["stream_id"] != capture_stream_id
            or capture_open.get("lease_id") != self.ids["lease_id"]
            or capture_open.get("turn_id") != self.ids["turn_id"]
            or capture_open.get("codec") != "pcm_s16le"
            or type(capture_open.get("sample_rate_hz")) is not int
            or capture_open["sample_rate_hz"] != 16000
            or type(capture_open.get("channels")) is not int
            or capture_open["channels"] != 1
            or type(capture_open.get("frame_duration_ms")) is not int
            or capture_open["frame_duration_ms"] != FRAME_DURATION_MS
        ):
            raise ValueError("capture-open stream does not match request")

        microphone_frames = 0
        microphone_pcm = bytearray()
        expected_sequence = 0
        previous_timestamp = -1
        while True:
            message = await self.receive(timeout=30.0)
            if isinstance(message, str):
                control = parse_control(message)
                if control["type"] == "audio.capture.close":
                    if (
                        type(control.get("stream_id")) is not int
                        or control["stream_id"] != capture_stream_id
                        or control.get("lease_id") != self.ids["lease_id"]
                        or control.get("turn_id") != self.ids["turn_id"]
                        or not isinstance(control.get("reason"), str)
                        or not control["reason"]
                    ):
                        raise ValueError("capture-close stream does not match request")
                    break
                continue
            header, payload = parse_media(message)
            if (
                header["kind"] != MEDIA_MICROPHONE_PCM
                or header["stream_id"] != capture_stream_id
                or header["sequence"] != expected_sequence
                or header["flags"] & ~0x0001
                or (
                    expected_sequence > 0
                    and header["timestamp_us"] <= previous_timestamp
                )
                or len(payload) != MICROPHONE_PAYLOAD_BYTES
                or microphone_frames >= MAX_MICROPHONE_FRAMES
            ):
                raise ValueError("microphone media frame is invalid")
            microphone_frames += 1
            microphone_pcm.extend(payload)
            expected_sequence += 1
            previous_timestamp = header["timestamp_us"]
        if microphone_frames == 0:
            raise ValueError("capture completed without microphone media")
        microphone_artifact = self.retain_pcm(
            "bodylink-microphone.pcm",
            bytes(microphone_pcm),
            "bodylink-microphone-pcm-s16le",
        )

        await self.send_control(
            "turn.state",
            lease_id=self.ids["lease_id"],
            turn_id=self.ids["turn_id"],
            state="thinking",
        )
        await self.send_control(
            "face.set",
            lease_id=self.ids["lease_id"],
            turn_id=self.ids["turn_id"],
            state_version=1,
            expression="thinking",
            caption=f"BOBE {self.evidence.manifest['challenges']['visual'][:8]}",
            accent_rgb=0x94472E,
            ttl_ms=10000,
        )

        playback_stream_id = 0xB0BE
        await self.send_control(
            "audio.playback.open",
            lease_id=self.ids["lease_id"],
            turn_id=self.ids["turn_id"],
            stream_id=playback_stream_id,
            codec="pcm_s16le",
            sample_rate_hz=SPEAKER_SAMPLE_RATE,
            channels=1,
            frame_duration_ms=FRAME_DURATION_MS,
        )
        ready = await self.receive_control("audio.playback.ready")
        if (
            type(ready.get("stream_id")) is not int
            or ready["stream_id"] != playback_stream_id
            or ready.get("lease_id") != self.ids["lease_id"]
            or ready.get("turn_id") != self.ids["turn_id"]
        ):
            raise ValueError("playback-ready stream is invalid")
        credit_ms = ready.get("credit_ms")
        if type(credit_ms) is not int or not 60 <= credit_ms <= MAX_PLAYBACK_CREDIT_MS:
            raise ValueError("playback-ready credit is invalid")

        capacity_ms = credit_ms
        frames = speaker_pcm(self.config.nonce)
        speaker_artifact = self.retain_pcm(
            "bodylink-speaker.pcm",
            b"".join(frames),
            "bodylink-speaker-pcm-s16le",
        )
        sent_samples = 0
        played_samples = 0
        sequence = 0
        while sequence < len(frames):
            outstanding_ms = (
                (sent_samples - played_samples) * 1000 // SPEAKER_SAMPLE_RATE
            )
            if (FRAME_DURATION_MS <= credit_ms
                    and outstanding_ms + FRAME_DURATION_MS <= capacity_ms):
                header = MEDIA_HEADER.pack(
                    PROTOCOL_MAJOR,
                    MEDIA_SPEAKER_PCM,
                    0,
                    playback_stream_id,
                    sequence,
                    sequence * FRAME_DURATION_MS * 1000,
                )
                packet = header + frames[sequence]
                await self.websocket.send(packet)
                self.evidence.event(
                    "bodylink.media",
                    direction="to-device",
                    header={
                        "kind": MEDIA_SPEAKER_PCM,
                        "stream_id": playback_stream_id,
                        "sequence": sequence,
                    },
                    payload_size=SPEAKER_PAYLOAD_BYTES,
                    payload_sha256=hashlib.sha256(frames[sequence]).hexdigest(),
                )
                sent_samples += SPEAKER_SAMPLES_PER_FRAME
                credit_ms -= FRAME_DURATION_MS
                sequence += 1
                continue

            ack = await self.receive_control("audio.playback.ack", timeout=30.0)
            if (
                type(ack.get("stream_id")) is not int
                or ack["stream_id"] != playback_stream_id
                or ack.get("lease_id") != self.ids["lease_id"]
                or ack.get("turn_id") != self.ids["turn_id"]
            ):
                raise ValueError("playback acknowledgement stream is invalid")
            acknowledged = ack.get("played_samples")
            next_credit = ack.get("credit_ms")
            if (
                type(acknowledged) is not int
                or acknowledged < played_samples
                or acknowledged > sent_samples
                or type(next_credit) is not int
                or not 0 <= next_credit <= capacity_ms
            ):
                raise ValueError("playback acknowledgement is invalid")
            played_samples = acknowledged
            credit_ms = next_credit

        await self.send_control(
            "audio.playback.close",
            lease_id=self.ids["lease_id"],
            turn_id=self.ids["turn_id"],
            stream_id=playback_stream_id,
        )
        terminal = await self.receive_control("audio.playback.drained", timeout=30.0)
        if (
            type(terminal.get("stream_id")) is not int
            or terminal["stream_id"] != playback_stream_id
            or terminal.get("lease_id") != self.ids["lease_id"]
            or terminal.get("turn_id") != self.ids["turn_id"]
        ):
            raise ValueError("playback terminal stream is invalid")
        await self.send_control(
            "turn.state",
            lease_id=self.ids["lease_id"],
            turn_id=self.ids["turn_id"],
            state="idle",
        )
        self.evidence.event(
            "bodylink.voice-roundtrip",
            microphone_frames=microphone_frames,
            speaker_frames=len(frames),
            speaker_samples=sent_samples,
            microphone_pcm_sha256=microphone_artifact["sha256"],
            speaker_pcm_sha256=speaker_artifact["sha256"],
        )

    async def run(self) -> None:
        await self.handshake()
        await self.ping()
        if self.config.scenario == "voice-roundtrip":
            await self.voice_roundtrip()


def server_ssl_context() -> ssl.SSLContext:
    fixed_private_file(SERVER_CERT_PATH, "BodyLink server certificate")
    fixed_private_file(SERVER_KEY_PATH, "BodyLink server private key")
    fixed_private_file(CA_CERT_PATH, "BodyLink CA certificate")
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(SERVER_CERT_PATH, SERVER_KEY_PATH)
    context.load_verify_locations(CA_CERT_PATH)
    context.verify_mode = ssl.CERT_REQUIRED
    return context


async def serve_once(
    config: BodyLinkConfig,
    evidence: EvidenceRun,
    host: str = "0.0.0.0",
    port: int = 8767,
    timeout: float = 180.0,
) -> None:
    completed: asyncio.Future[None] = asyncio.get_running_loop().create_future()

    async def handler(connection: ServerConnection) -> None:
        if connection.subprotocol != SUBPROTOCOL:
            raise ValueError("BodyLink WebSocket subprotocol was not negotiated")
        ssl_object = connection.transport.get_extra_info("ssl_object")
        peer = ssl_object.getpeercert() if ssl_object is not None else None
        common_names = {
            value
            for sequence in (peer or {}).get("subject", ())
            for key, value in sequence
            if key == "commonName"
        }
        if config.expected_device_id not in common_names:
            raise ValueError("mTLS client certificate does not match device identity")
        try:
            await BodyLinkSession(connection, config, evidence).run()
            if not completed.done():
                completed.set_result(None)
        except Exception as error:
            if not completed.done():
                completed.set_exception(error)
            raise

    async with serve(
        handler,
        host,
        port,
        ssl=server_ssl_context(),
        subprotocols=[Subprotocol(SUBPROTOCOL)],
        max_size=MAX_CONTROL_BYTES,
        ping_interval=10,
        ping_timeout=10,
    ):
        await asyncio.wait_for(completed, timeout=timeout)
