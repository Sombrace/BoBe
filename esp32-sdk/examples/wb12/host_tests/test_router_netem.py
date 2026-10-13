import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
ROUTER_SCRIPT = ROOT / "hil" / "router" / "bobe-netem"


class RouterNetemTests(unittest.TestCase):
    def write_command(self, directory: Path, name: str, source: str) -> None:
        path = directory / name
        path.write_text(source, encoding="utf-8")
        path.chmod(0o755)

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.state = self.root / "state"
        self.config = self.root / "netem.conf"
        self.config.write_text(
            "TEST_INTERFACE=test0\nMANAGEMENT_INTERFACE=mgmt0\n",
            encoding="ascii",
        )
        self.script = self.root / "bobe-netem"
        source = ROUTER_SCRIPT.read_text(encoding="utf-8")
        source = source.replace(
            "CONFIG=/etc/bobe-hil/netem.conf",
            f"CONFIG={self.config}",
        ).replace(
            "STATE_DIR=/var/run/bobe-hil",
            f"STATE_DIR={self.state}",
        )
        self.script.write_text(source, encoding="utf-8")
        self.script.chmod(0o755)

        self.write_command(self.bin, "ip", "#!/bin/sh\nexit 0\n")
        self.write_command(
            self.bin,
            "nohup",
            "#!/bin/sh\nexit 0\n",
        )
        self.write_command(
            self.bin,
            "tc",
            """#!/bin/sh
state="$FAKE_NETEM_ROOT/tc.state"
if [ "$1" = "-j" ]; then
    printf '[]\\n'
elif [ "$1" = "qdisc" ] && [ "$2" = "show" ]; then
    [ "${FAIL_TC_PROBE:-0}" != 1 ] || exit 1
    if [ -f "$state" ]; then
        cat "$state"
    else
        printf 'qdisc noqueue 0: root refcnt 2\\n'
    fi
elif [ "$1" = "qdisc" ] && [ "$2" = "replace" ]; then
    printf 'qdisc netem 8001: root refcnt 2\\n' >"$state"
elif [ "$1" = "qdisc" ] && [ "$2" = "del" ]; then
    [ "${FAIL_TC_CLEAR:-0}" != 1 ] || exit 1
    rm -f "$state"
else
    exit 2
fi
""",
        )
        self.write_command(
            self.bin,
            "nft",
            """#!/bin/sh
state="$FAKE_NETEM_ROOT/nft.state"
if [ "$1" = "list" ] && [ "$2" = "tables" ]; then
    if [ -f "$state" ]; then
        printf 'table inet bobe_hil\\n'
    fi
    exit 0
elif [ "$1" = "delete" ]; then
    [ "${FAIL_NFT_CLEAR:-0}" != 1 ] || exit 1
    rm -f "$state"
elif [ "$1" = "-f" ] && [ "$2" = "-" ]; then
    cat >/dev/null
    : >"$state"
else
    exit 2
fi
""",
        )
        self.environment = {
            **os.environ,
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "FAKE_NETEM_ROOT": str(self.root),
        }

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_netem(
        self,
        *arguments: str,
        environment: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(self.script), *arguments],
            check=False,
            capture_output=True,
            text=True,
            env=environment or self.environment,
        )

    def test_rejects_replacement_and_retains_lease_on_failed_clear(self) -> None:
        first = "a" * 32
        second = "b" * 32
        applied = self.run_netem("apply", first, "loss-1", "30")
        self.assertEqual(applied.returncode, 0, applied.stderr)
        lease = self.state / "netem.lease"
        self.assertTrue(lease.read_text().startswith(f"{first} loss-1 "))

        replacement = self.run_netem("apply", second, "loss-5", "30")
        self.assertNotEqual(replacement.returncode, 0)
        self.assertIn("ERR BUSY", replacement.stderr)
        self.assertTrue(lease.read_text().startswith(f"{first} loss-1 "))

        probe_environment = {**self.environment, "FAIL_TC_PROBE": "1"}
        failed_status = self.run_netem(
            "status",
            environment=probe_environment,
        )
        self.assertNotEqual(failed_status.returncode, 0)
        self.assertTrue(lease.is_file())

        failed_environment = {**self.environment, "FAIL_TC_CLEAR": "1"}
        failed_clear = self.run_netem(
            "clear",
            first,
            environment=failed_environment,
        )
        self.assertNotEqual(failed_clear.returncode, 0)
        self.assertTrue(lease.is_file())
        self.assertTrue((self.root / "tc.state").is_file())

        cleared = self.run_netem("clear", first)
        self.assertEqual(cleared.returncode, 0, cleared.stderr)
        self.assertFalse(lease.exists())
        self.assertFalse((self.root / "tc.state").exists())

    def test_blackhole_is_verified_and_cleared(self) -> None:
        lease = "c" * 32
        applied = self.run_netem("apply", lease, "blackhole", "30")
        self.assertEqual(applied.returncode, 0, applied.stderr)
        self.assertTrue((self.root / "nft.state").is_file())
        status = self.run_netem("status")
        self.assertEqual(status.returncode, 0, status.stderr)
        self.assertIn("profile=blackhole", status.stdout)
        cleared = self.run_netem("clear", lease)
        self.assertEqual(cleared.returncode, 0, cleared.stderr)
        self.assertFalse((self.root / "nft.state").exists())


if __name__ == "__main__":
    unittest.main()
