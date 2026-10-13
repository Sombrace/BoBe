import unittest

from bobe_hil.network_profiles import NetworkLease


class NetworkPolicyTests(unittest.TestCase):
    def test_builds_only_fixed_router_commands(self) -> None:
        lease = NetworkLease("ab" * 16, "delay-150", 90)
        self.assertEqual(
            lease.apply_arguments(),
            (
                "/usr/sbin/bobe-netem",
                "apply",
                "ab" * 16,
                "delay-150",
                "90",
            ),
        )
        self.assertEqual(
            lease.clear_arguments(),
            ("/usr/sbin/bobe-netem", "clear", "ab" * 16),
        )

    def test_rejects_raw_profiles_and_unbounded_ttl(self) -> None:
        with self.assertRaises(ValueError):
            NetworkLease("ab" * 16, "tc qdisc del dev br-lan", 30)
        with self.assertRaises(ValueError):
            NetworkLease("ab" * 16, "clean", 3600)
        with self.assertRaises(ValueError):
            NetworkLease("not-a-lease", "clean", 30)


if __name__ == "__main__":
    unittest.main()
