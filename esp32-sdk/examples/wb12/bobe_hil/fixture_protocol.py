import re
import time
from dataclasses import dataclass
from typing import Protocol


PROTOCOL_VERSION = 1
FIRMWARE_VERSION = "0.3.0"
MIN_PRESS_MS = 30
MAX_PRESS_MS = 2000
MIN_RESET_MS = 50
MAX_RESET_MS = 500
CONTROLS = {
    "action": "ACTION",
    "volume-down": "VOLUME_DOWN",
    "volume-up": "VOLUME_UP",
}
CHORDS = {
    "shutdown": "SHUTDOWN",
    "provision-recovery": "PROVISION_RECOVERY",
}
HELLO_PATTERN = re.compile(
    r"^OK HELLO 1 ([0-9a-f]{32}) ([0-9]+\.[0-9]+\.[0-9]+) "
    r"([0-9a-f]{16})$"
)
STATUS_PATTERN = re.compile(
    r"^OK STATUS ([0-9]+) ([0-9a-f]{32}) "
    r"mask=([0-9]+) reset=([0-9]+) loader=([0-9]+) "
    r"action_lockout=([01])$"
)


class FixtureTransport(Protocol):
    def exchange(self, line: str, timeout_seconds: float) -> str: ...


@dataclass(frozen=True)
class FixtureIdentity:
    protocol_version: int
    nonce: str
    firmware_version: str
    board_id: str


class FixtureClient:
    def __init__(
        self,
        transport: FixtureTransport,
        nonce: str,
        expected_board_id: str,
    ) -> None:
        if len(nonce) != 32 or any(character not in "0123456789abcdef" for character in nonce):
            raise ValueError("fixture session nonce is malformed")
        if re.fullmatch(r"[0-9a-f]{16}", expected_board_id) is None:
            raise ValueError("expected fixture board identity is malformed")
        self.transport = transport
        self.nonce = nonce
        self.expected_board_id = expected_board_id
        self.sequence = 0
        self.identity: FixtureIdentity | None = None

    def hello(self) -> FixtureIdentity:
        response = self.transport.exchange(
            f"HELLO {PROTOCOL_VERSION} {self.nonce}",
            2.0,
        )
        match = HELLO_PATTERN.fullmatch(response)
        if (
            match is None
            or match.group(1) != self.nonce
            or match.group(2) != FIRMWARE_VERSION
            or match.group(3) != self.expected_board_id
        ):
            raise ValueError("fixture HELLO response is invalid")
        self.identity = FixtureIdentity(
            PROTOCOL_VERSION,
            match.group(1),
            match.group(2),
            match.group(3),
        )
        return self.identity

    def next_sequence(self) -> int:
        if self.identity is None:
            raise RuntimeError("fixture HELLO is required before commands")
        self.sequence += 1
        return self.sequence

    def command(self, operation: str, *arguments: str) -> str:
        sequence = self.next_sequence()
        line = " ".join(
            (
                operation,
                str(sequence),
                self.nonce,
                *arguments,
            )
        )
        response = self.transport.exchange(line, 3.0)
        expected = f"OK {operation} {sequence} {self.nonce}"
        if response != expected and not response.startswith(f"{expected} "):
            raise ValueError(
                f"fixture rejected {operation}: {response}"
            )
        return response

    def status(self) -> str:
        response = self.command("STATUS")
        self.parse_status(response)
        return response

    def parse_status(self, response: str) -> re.Match[str]:
        match = STATUS_PATTERN.fullmatch(response)
        if (
            match is None
            or match.group(2) != self.nonce
            or int(match.group(1)) != self.sequence
            or int(match.group(3)) > 0x1F
            or int(match.group(4)) > 2
            or int(match.group(5)) > 3
        ):
            raise ValueError("fixture STATUS response is invalid")
        return match

    def wait_released(self, timeout_seconds: float) -> str:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            response = self.status()
            match = self.parse_status(response)
            if (
                int(match.group(3)) == 0
                and int(match.group(4)) == 0
                and int(match.group(5)) == 0
            ):
                return response
            time.sleep(0.05)
        raise TimeoutError("fixture outputs did not release")

    def press(self, control: str, duration_ms: int) -> str:
        if control not in CONTROLS:
            raise ValueError("fixture control is unsupported")
        if not MIN_PRESS_MS <= duration_ms <= MAX_PRESS_MS:
            raise ValueError("fixture press duration is outside the bounded range")
        self.command(
            "PRESS",
            CONTROLS[control],
            str(duration_ms),
        )
        return self.wait_released(duration_ms / 1000.0 + 1.0)

    def chord(self, chord: str, duration_ms: int) -> str:
        if chord not in CHORDS:
            raise ValueError("fixture chord is unsupported")
        if not MIN_PRESS_MS <= duration_ms <= MAX_PRESS_MS:
            raise ValueError("fixture chord duration is outside the bounded range")
        self.command(
            "CHORD",
            CHORDS[chord],
            str(duration_ms),
        )
        return self.wait_released(duration_ms / 1000.0 + 1.0)

    def reset(self, duration_ms: int = 120) -> str:
        if not MIN_RESET_MS <= duration_ms <= MAX_RESET_MS:
            raise ValueError("fixture reset duration is outside the bounded range")
        self.command("RESET", str(duration_ms))
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            status = self.status()
            match = self.parse_status(status)
            if (
                int(match.group(3)) == 0
                and int(match.group(4)) == 0
                and int(match.group(5)) == 0
                and match.group(6) == "0"
            ):
                return status
            time.sleep(0.05)
        raise TimeoutError("fixture reset did not complete")

    def enter_loader(self) -> str:
        self.command("ENTER_LOADER")
        return self.wait_released(2.0)

    def release_all(self) -> str:
        return self.command("RELEASE_ALL")
