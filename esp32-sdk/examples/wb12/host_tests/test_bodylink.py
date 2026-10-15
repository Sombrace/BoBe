import asyncio
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bobe_hil import bodylink, evidence


class FakeWebSocket:
    def __init__(self, incoming: list[str | bytes]) -> None:
        self.incoming = list(incoming)
        self.sent: list[str | bytes] = []

    async def recv(self) -> str | bytes:
        if not self.incoming:
            raise RuntimeError("fake websocket has no queued message")
        return self.incoming.pop(0)

    async def send(self, message: str | bytes) -> None:
        self.sent.append(message)


class VoiceWebSocket(FakeWebSocket):
    def __init__(
        self,
        incoming: list[str | bytes],
        playback_stream_id: int,
        lease_id: str,
        turn_id: str,
    ) -> None:
        super().__init__(incoming)
        self.playback_stream_id = playback_stream_id
        self.lease_id = lease_id
        self.turn_id = turn_id
        self.speaker_frames = 0

    async def send(self, message: str | bytes) -> None:
        await super().send(message)
        if isinstance(message, bytes):
            header, _ = bodylink.parse_media(message)
            self.speaker_frames += 1
            if self.speaker_frames % 6 == 0:
                self.incoming.append(
                    bodylink.control_message(
                        "audio.playback.ack",
                        lease_id=self.lease_id,
                        turn_id=self.turn_id,
                        stream_id=self.playback_stream_id,
                        played_samples=(
                            self.speaker_frames
                            * bodylink.SPEAKER_SAMPLES_PER_FRAME
                        ),
                        credit_ms=120,
                    )
                )
            return
        control = bodylink.parse_control(message)
        if control["type"] == "audio.playback.close":
            self.incoming.append(
                bodylink.control_message(
                    "audio.playback.drained",
                    lease_id=control["lease_id"],
                    turn_id=control["turn_id"],
                    stream_id=self.playback_stream_id,
                )
            )


class BodyLinkTests(unittest.TestCase):
    def evidence_run(self, root: Path) -> evidence.EvidenceRun:
        patcher = mock.patch.object(evidence, "RUNS_ROOT", root / "runs")
        patcher.start()
        self.addCleanup(patcher.stop)
        run = evidence.EvidenceRun.create("bodylink-test")
        self.addCleanup(run.close)
        return run

    def test_handshake_waits_for_session_health(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            nonce = "ab" * 16
            config = bodylink.BodyLinkConfig(
                "device-1",
                7,
                "handshake",
                nonce,
            )
            identifiers = bodylink.challenge_ids(nonce)
            hello = bodylink.control_message(
                "hello",
                protocol_minor=1,
                device_id="device-1",
                boot_id="1" * 32,
            )
            health = bodylink.control_message("health.report", build_number=1)
            websocket = FakeWebSocket([hello, health])
            run = self.evidence_run(Path(directory))
            asyncio.run(
                bodylink.BodyLinkSession(websocket, config, run).run()
            )
            sent = [
                json.loads(message)
                for message in websocket.sent
                if isinstance(message, str)
            ]
            self.assertEqual(
                [message["type"] for message in sent],
                ["welcome"],
            )
            self.assertEqual(sent[0]["controller_epoch"], 7)
            self.assertEqual(sent[0]["session_id"], identifiers["session_id"])
            self.assertEqual(sent[0]["heartbeat_interval_ms"], 10000)
            self.assertEqual(websocket.incoming, [])

    def test_rejects_wrong_hello_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            config = bodylink.BodyLinkConfig(
                "device-1",
                1,
                "handshake",
                "01" * 16,
            )
            hello = bodylink.control_message(
                "hello",
                protocol_minor=1,
                device_id="device-2",
                boot_id="1" * 32,
            )
            run = self.evidence_run(Path(directory))
            with self.assertRaisesRegex(ValueError, "identity"):
                asyncio.run(
                    bodylink.BodyLinkSession(
                        FakeWebSocket([hello]),
                        config,
                        run,
                    ).handshake()
                )

    def test_rejects_media_while_waiting_for_control(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            config = bodylink.BodyLinkConfig(
                "device-1",
                1,
                "handshake",
                "02" * 16,
            )
            unexpected = (
                bodylink.MEDIA_HEADER.pack(1, 1, 0, 1, 0, 1)
                + bytes(bodylink.MICROPHONE_PAYLOAD_BYTES)
            )
            run = self.evidence_run(Path(directory))
            session = bodylink.BodyLinkSession(
                FakeWebSocket([unexpected]),
                config,
                run,
            )
            with self.assertRaisesRegex(ValueError, "unexpected BodyLink media"):
                asyncio.run(session.receive_control("health.report"))

    def test_speaker_challenge_is_deterministic_and_bounded(self) -> None:
        first = bodylink.speaker_pcm("12" * 16, frame_count=3)
        second = bodylink.speaker_pcm("12" * 16, frame_count=3)
        self.assertEqual(first, second)
        self.assertEqual(len(first), 3)
        self.assertTrue(
            all(
                len(frame) == bodylink.SPEAKER_PAYLOAD_BYTES
                for frame in first
            )
        )
        self.assertNotEqual(
            first,
            bodylink.speaker_pcm("34" * 16, frame_count=3),
        )

    def test_media_parser_rejects_bad_shape(self) -> None:
        with self.assertRaises(ValueError):
            bodylink.parse_media(b"short")
        header = bodylink.MEDIA_HEADER.pack(2, 1, 0, 1, 0, 0)
        with self.assertRaises(ValueError):
            bodylink.parse_media(header + b"payload")
        with self.assertRaisesRegex(ValueError, "protocol major"):
            bodylink.parse_control(
                json.dumps({"type": "hello", "protocol_major": True})
            )

    def test_control_rejects_ambiguous_and_non_json_values(self) -> None:
        for message in [
            '{"type":"health.report","protocol_major":1,"protocol_major":1}',
            '{"type":"health.report","protocol_major":1,"uptime_s":NaN}',
            '{"type":"health.report","protocol_major":1,"nested":{"value":1,"value":2}}',
        ]:
            with self.subTest(message=message), self.assertRaises(ValueError):
                bodylink.parse_control(message)

    def test_media_parser_matches_firmware_bounds(self) -> None:
        for kind, flags, length in [(0, 0, 640), (1, 2, 640), (1, 0, 638),
                                    (2, 0, 962), (3, 0, 1), (4, 0, 0)]:
            packet = bodylink.MEDIA_HEADER.pack(1, kind, flags, 1, 0, 0) + bytes(length)
            with self.subTest(kind=kind, flags=flags, length=length), self.assertRaises(ValueError):
                bodylink.parse_media(packet)
        for kind, length in [(1, 640), (2, 960)]:
            packet = bodylink.MEDIA_HEADER.pack(1, kind, 1, 1, 0, 0) + bytes(length)
            header, payload = bodylink.parse_media(packet)
            self.assertEqual(header["kind"], kind)
            self.assertEqual(len(payload), length)

    def test_voice_roundtrip_obeys_playback_credit(self) -> None:
        for credit_ms in (120, bodylink.MAX_PLAYBACK_CREDIT_MS):
            with self.subTest(credit_ms=credit_ms):
                self.check_voice_roundtrip(credit_ms)

    def check_voice_roundtrip(self, credit_ms: int) -> None:
        with tempfile.TemporaryDirectory() as directory:
            nonce = "78" * 16
            config = bodylink.BodyLinkConfig(
                "device-1",
                1,
                "voice-roundtrip",
                nonce,
            )
            identifiers = bodylink.challenge_ids(nonce)
            capture_stream = 17
            playback_stream = 0xB0BE
            hello = bodylink.control_message(
                "hello",
                protocol_minor=1,
                device_id="device-1",
                boot_id="1" * 32,
            )
            request = bodylink.control_message(
                "capture.request",
                request_id="request-1",
                stream_id=capture_stream,
                trigger="action_button",
                codec="pcm_s16le",
                sample_rate_hz=16000,
                channels=1,
                frame_duration_ms=20,
            )
            capture_open = bodylink.control_message(
                "audio.capture.open",
                lease_id=identifiers["lease_id"],
                turn_id=identifiers["turn_id"],
                stream_id=capture_stream,
                codec="pcm_s16le",
                sample_rate_hz=16000,
                channels=1,
                frame_duration_ms=20,
            )
            microphone = (
                bodylink.MEDIA_HEADER.pack(
                    1,
                    bodylink.MEDIA_MICROPHONE_PCM,
                    0,
                    capture_stream,
                    0,
                    1,
                )
                + bytes(bodylink.MICROPHONE_PAYLOAD_BYTES)
            )
            capture_close = bodylink.control_message(
                "audio.capture.close",
                lease_id=identifiers["lease_id"],
                turn_id=identifiers["turn_id"],
                stream_id=capture_stream,
                reason="button_released",
            )
            ready = bodylink.control_message(
                "audio.playback.ready",
                lease_id=identifiers["lease_id"],
                turn_id=identifiers["turn_id"],
                stream_id=playback_stream,
                credit_ms=credit_ms,
            )
            websocket = VoiceWebSocket(
                [
                    hello,
                    request,
                    capture_open,
                    microphone,
                    capture_close,
                    ready,
                ],
                playback_stream,
                str(identifiers["lease_id"]),
                str(identifiers["turn_id"]),
            )
            run = self.evidence_run(Path(directory))
            asyncio.run(
                bodylink.BodyLinkSession(websocket, config, run).run()
            )
            self.assertEqual(websocket.speaker_frames, 150)
            controls = [
                json.loads(message)["type"]
                for message in websocket.sent
                if isinstance(message, str)
            ]
            self.assertEqual(
                controls[-2:],
                ["audio.playback.close", "turn.state"],
            )
            artifacts = {
                artifact["name"]: artifact
                for artifact in run.manifest["artifacts"]
            }
            self.assertEqual(
                set(artifacts),
                {
                    "bodylink-microphone.pcm",
                    "bodylink-speaker.pcm",
                },
            )
            self.assertEqual(
                artifacts["bodylink-microphone.pcm"]["size"],
                bodylink.MICROPHONE_PAYLOAD_BYTES,
            )
            self.assertEqual(
                artifacts["bodylink-speaker.pcm"]["size"],
                150 * bodylink.SPEAKER_PAYLOAD_BYTES,
            )
            for artifact in artifacts.values():
                path = run.directory / artifact["path"]
                self.assertTrue(path.is_file())
                self.assertEqual(path.stat().st_size, artifact["size"])
            events = (run.directory / "events.jsonl").read_text()
            self.assertIn(
                artifacts["bodylink-microphone.pcm"]["sha256"],
                events,
            )
            self.assertIn(
                artifacts["bodylink-speaker.pcm"]["sha256"],
                events,
            )

    def test_voice_capture_rejects_more_than_ten_seconds(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            nonce = "90" * 16
            config = bodylink.BodyLinkConfig(
                "device-1",
                1,
                "voice-roundtrip",
                nonce,
            )
            identifiers = bodylink.challenge_ids(nonce)
            stream_id = 17
            incoming: list[str | bytes] = [
                bodylink.control_message(
                    "capture.request",
                    request_id="request-1",
                    stream_id=stream_id,
                    trigger="action_button",
                    codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    frame_duration_ms=20,
                ),
                bodylink.control_message(
                    "audio.capture.open",
                    lease_id=identifiers["lease_id"],
                    turn_id=identifiers["turn_id"],
                    stream_id=stream_id,
                    codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    frame_duration_ms=20,
                ),
            ]
            incoming.extend(
                bodylink.MEDIA_HEADER.pack(
                    1,
                    bodylink.MEDIA_MICROPHONE_PCM,
                    0,
                    stream_id,
                    sequence,
                    sequence + 1,
                )
                + bytes(bodylink.MICROPHONE_PAYLOAD_BYTES)
                for sequence in range(bodylink.MAX_MICROPHONE_FRAMES + 1)
            )
            run = self.evidence_run(Path(directory))
            with self.assertRaisesRegex(ValueError, "microphone media frame"):
                asyncio.run(
                    bodylink.BodyLinkSession(
                        FakeWebSocket(incoming),
                        config,
                        run,
                    ).voice_roundtrip()
                )
            self.assertEqual(run.manifest["artifacts"], [])


if __name__ == "__main__":
    unittest.main()
