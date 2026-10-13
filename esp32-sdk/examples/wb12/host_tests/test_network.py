import json
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

from bobe_hil import network


class FakeOpenWrtTransport:
    def __init__(self) -> None:
        self.lease: str | None = None
        self.profile = "clean"
        self.expires = 0
        self.calls: list[tuple[str, ...]] = []
        self.invalid_apply_response = False

    def run(self, arguments: tuple[str, ...]) -> str:
        self.calls.append(arguments)
        if arguments == ("status",):
            return (
                "OK STATUS lease=none profile=clean expires=0\n[]"
                if self.lease is None
                else (
                    f"OK STATUS lease={self.lease} "
                    f"profile={self.profile} expires={self.expires}\n[]"
                )
            )
        if arguments[0] == "apply":
            self.lease = arguments[1]
            self.profile = arguments[2]
            self.expires = int(time.time()) + int(arguments[3])
            if self.invalid_apply_response:
                return "OK APPLY malformed"
            return (
                f"OK APPLY lease={self.lease} "
                f"profile={self.profile} expires={self.expires}"
            )
        if arguments[0] == "clear" and arguments[1] == self.lease:
            lease = self.lease
            self.lease = None
            self.profile = "clean"
            self.expires = 0
            return f"OK CLEAR lease={lease}"
        raise RuntimeError("unexpected fake OpenWrt operation")


class NetworkTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.write_config(self.root)
        self.transport = FakeOpenWrtTransport()
        self.transport_class = network.OpenWrtTransport
        patcher = mock.patch.object(
            network,
            "OpenWrtTransport",
            return_value=self.transport,
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def real_transport(self, config: object) -> object:
        return self.transport_class(config)

    def write_config(self, root: Path) -> Path:
        network_root = root / "network"
        network_root.mkdir()
        (network_root / "openwrt.json").write_text(
            json.dumps({"schema_version": 1, "host": "192.168.50.1"}),
            encoding="utf-8",
        )
        (network_root / "known_hosts").write_text(
            "[192.168.50.1]:22 ssh-ed25519 AAAA\n",
            encoding="ascii",
        )
        (network_root / "router_ed25519").write_text("private", encoding="ascii")
        (network_root / "openwrt.json").chmod(0o644)
        (network_root / "known_hosts").chmod(0o644)
        (network_root / "router_ed25519").chmod(0o600)
        return network_root

    def test_ssh_transport_pins_host_key_and_identity_files(self) -> None:
        config = network.load_network_config(self.root)
        transport = self.real_transport(config)
        command = " ".join(transport.command)
        self.assertIn("StrictHostKeyChecking=yes", command)
        self.assertIn(str(self.root / "network" / "known_hosts"), command)
        self.assertIn(str(self.root / "network" / "router_ed25519"), command)
        self.assertIn("root@192.168.50.1", command)
        self.assertIn("/usr/sbin/bobe-netem", command)

    def test_group_readable_private_key_is_rejected(self) -> None:
        (self.root / "network" / "router_ed25519").chmod(0o640)
        with self.assertRaises(ValueError) as error:
            network.load_network_config(self.root)
        self.assertIn("OpenWrt SSH key", str(error.exception))

    def test_apply_uses_caller_profile_and_ttl(self) -> None:
        result = network.apply("loss-5", 45, root=self.root)
        self.assertEqual(result["profile"], "loss-5")
        self.assertEqual(result["ttl_seconds"], 45)
        applied = [call for call in self.transport.calls if call[0] == "apply"]
        self.assertEqual(applied[0][2], "loss-5")
        self.assertEqual(applied[0][3], "45")
        state = json.loads(
            (self.root / "network" / "active-lease.json").read_text(encoding="utf-8")
        )
        self.assertEqual(state["phase"], "active")
        self.assertEqual(state["profile"], "loss-5")

    def test_ttl_outside_router_bounds_is_rejected(self) -> None:
        for ttl in (4, 601, "60"):
            with self.assertRaises(ValueError):
                network.apply("blackhole", ttl, root=self.root)
        self.assertEqual(self.transport.calls, [])

    def test_unknown_profile_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            network.apply("firehose", 60, root=self.root)

    def test_clean_profile_clears_the_active_lease(self) -> None:
        network.apply("blackhole", 30, root=self.root)
        result = network.apply("clean", 30, root=self.root)
        self.assertTrue(result["cleared"])
        self.assertFalse((self.root / "network" / "active-lease.json").exists())
        self.assertIsNone(self.transport.lease)

    def test_clear_without_a_lease_is_reported_not_raised(self) -> None:
        result = network.clear(self.root)
        self.assertFalse(result["cleared"])
        self.assertEqual(result["reason"], "no-active-lease")

    def test_second_apply_refuses_to_stack_leases(self) -> None:
        network.apply("delay-150", 30, root=self.root)
        with self.assertRaises(RuntimeError):
            network.apply("loss-1", 30, root=self.root)

    def test_malformed_apply_response_rolls_back_the_router(self) -> None:
        self.transport.invalid_apply_response = True
        with self.assertRaises(RuntimeError):
            network.apply("blackhole", 30, root=self.root)
        self.assertIsNone(self.transport.lease)
        self.assertFalse((self.root / "network" / "active-lease.json").exists())

    def test_status_reconciles_pending_local_state(self) -> None:
        network_root = self.root / "network"
        lease = "a" * 32
        self.transport.lease = lease
        self.transport.profile = "blackhole"
        self.transport.expires = int(time.time()) + 60
        (network_root / "active-lease.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "lease": lease,
                    "profile": "blackhole",
                    "expires": 0,
                    "phase": "pending",
                }
            ),
            encoding="utf-8",
        )
        (network_root / "active-lease.json").chmod(0o600)
        result = network.status(self.root)
        self.assertEqual(result["local"]["phase"], "active")
        self.assertEqual(result["remote"]["lease"], lease)

    def test_unowned_router_lease_is_surfaced(self) -> None:
        self.transport.lease = "b" * 32
        self.transport.profile = "blackhole"
        self.transport.expires = int(time.time()) + 60
        with self.assertRaises(RuntimeError) as error:
            network.status(self.root)
        self.assertIn("did not record", str(error.exception))

    def test_inspect_reports_missing_configuration_as_fact(self) -> None:
        with tempfile.TemporaryDirectory() as empty:
            report = network.inspect_network(Path(empty))
        self.assertFalse(report["configured"])
        self.assertFalse(report["available"])
        self.assertNotIn("qualified", json.dumps(report))


if __name__ == "__main__":
    unittest.main()
