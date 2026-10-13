import unittest

from bobe_hil.fixture_protocol import FixtureClient


class FakeTransport:
    def __init__(
        self,
        nonce: str,
        *,
        firmware_version: str = "0.3.0",
        board_id: str = "0123456789abcdef",
    ) -> None:
        self.nonce = nonce
        self.firmware_version = firmware_version
        self.board_id = board_id
        self.lines: list[str] = []

    def exchange(self, line: str, timeout_seconds: float) -> str:
        self.lines.append(line)
        self.assert_timeout(timeout_seconds)
        fields = line.split()
        if fields[0] == "HELLO":
            return (
                f"OK HELLO 1 {self.nonce} {self.firmware_version} "
                f"{self.board_id}"
            )
        if fields[0] == "STATUS":
            return (
                f"OK STATUS {fields[1]} {fields[2]} "
                "mask=0 reset=0 loader=0 action_lockout=0"
            )
        return f"OK {fields[0]} {fields[1]} {fields[2]}"

    @staticmethod
    def assert_timeout(timeout_seconds: float) -> None:
        if not 0 < timeout_seconds <= 3:
            raise AssertionError("fixture command timeout is not bounded")


class FixtureProtocolTests(unittest.TestCase):
    def test_only_semantic_bounded_commands_are_emitted(self) -> None:
        nonce = "12" * 16
        transport = FakeTransport(nonce)
        client = FixtureClient(transport, nonce, "0123456789abcdef")
        identity = client.hello()
        self.assertEqual(identity.board_id, "0123456789abcdef")
        client.status()
        client.press("action", 120)
        client.chord("provision-recovery", 1300)
        client.reset()
        client.enter_loader()
        client.release_all()
        self.assertEqual(
            transport.lines,
            [
                f"HELLO 1 {nonce}",
                f"STATUS 1 {nonce}",
                f"PRESS 2 {nonce} ACTION 120",
                f"STATUS 3 {nonce}",
                f"CHORD 4 {nonce} PROVISION_RECOVERY 1300",
                f"STATUS 5 {nonce}",
                f"RESET 6 {nonce} 120",
                f"STATUS 7 {nonce}",
                f"ENTER_LOADER 8 {nonce}",
                f"STATUS 9 {nonce}",
                f"RELEASE_ALL 10 {nonce}",
            ],
        )

    def test_rejects_arbitrary_controls_and_durations(self) -> None:
        nonce = "34" * 16
        client = FixtureClient(
            FakeTransport(nonce),
            nonce,
            "0123456789abcdef",
        )
        client.hello()
        with self.assertRaises(ValueError):
            client.press("gpio-17", 100)
        with self.assertRaises(ValueError):
            client.press("action", 5000)
        with self.assertRaises(ValueError):
            client.reset(10)

    def test_requires_hello_before_actuation(self) -> None:
        nonce = "56" * 16
        client = FixtureClient(
            FakeTransport(nonce),
            nonce,
            "0123456789abcdef",
        )
        with self.assertRaises(RuntimeError):
            client.press("action", 120)

    def test_rejects_unexpected_fixture_identity_or_firmware(self) -> None:
        nonce = "78" * 16
        for transport in (
            FakeTransport(nonce, firmware_version="0.2.0"),
            FakeTransport(nonce, board_id="fedcba9876543210"),
        ):
            client = FixtureClient(
                transport,
                nonce,
                "0123456789abcdef",
            )
            with self.assertRaisesRegex(ValueError, "HELLO"):
                client.hello()


if __name__ == "__main__":
    unittest.main()
