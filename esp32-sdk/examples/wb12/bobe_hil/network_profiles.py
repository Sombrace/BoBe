import re
from dataclasses import dataclass


NETWORK_PROFILES = {
    "clean",
    "delay-150",
    "loss-1",
    "loss-5",
    "jitter-loss",
    "rate-512",
    "blackhole",
}
LEASE_PATTERN = re.compile(r"^[0-9a-f]{32}$")


@dataclass(frozen=True)
class NetworkLease:
    lease: str
    profile: str
    ttl_seconds: int

    def __post_init__(self) -> None:
        if LEASE_PATTERN.fullmatch(self.lease) is None:
            raise ValueError("network lease identifier is malformed")
        if self.profile not in NETWORK_PROFILES:
            raise ValueError("network impairment profile is unsupported")
        if not 5 <= self.ttl_seconds <= 600:
            raise ValueError("network impairment TTL is outside the bounded range")

    def apply_arguments(self) -> tuple[str, ...]:
        return (
            "/usr/sbin/bobe-netem",
            "apply",
            self.lease,
            self.profile,
            str(self.ttl_seconds),
        )

    def clear_arguments(self) -> tuple[str, ...]:
        return (
            "/usr/sbin/bobe-netem",
            "clear",
            self.lease,
        )
