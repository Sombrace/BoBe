# Isolated OpenWrt test LAN

The selected reference router is OpenWrt One with official OpenWrt 25.12.5,
profile `openwrt_one`, target `mediatek/filogic`. The pinned sysupgrade image and
SHA-256 are in `openwrt-one.lock`.

## Topology

- OpenWrt One 1G LAN -> Mac USB Ethernet.
- Mac Ethernet uses a static address on the management subnet with no gateway
  and no DNS.
- Mac internet/default route remains its existing Wi-Fi.
- The 2.4 GHz radio is the WB-12 test interface.
- Management traffic never traverses the impaired interface.

## Named impairment appliance

Install `bobe-netem` as `/usr/sbin/bobe-netem` and create a root-owned
`/etc/bobe-hil/netem.conf` from the example. It accepts only:

```text
bobe-netem status
bobe-netem apply LEASE clean|delay-150|loss-1|loss-5|jitter-loss|rate-512|blackhole TTL
bobe-netem clear LEASE
```

TTL is 5-600 seconds. A router-side background expiry clears qdisc/nft state
even if the host process dies. The script refuses to use the configured wired
management interface as the test interface. It rejects an existing lease or a
foreign root qdisc, applies nft blackholes as one batch, verifies every profile,
and retains the lease whenever rollback fails; cleanup is never reported as
successful until qdisc/nft state is confirmed clean.

`bobe-netem` is the only remote command this repository invokes.

## Host transport

Install these files under the gitignored repository-local path:

```text
private/hil/network/
```

- `openwrt.json`, based on `network-config.example.json`, with a fixed private
  management IPv4 address;
- `known_hosts`, pinned to that exact address and OpenWrt SSH host key;
- `router_ed25519`, mode 0600.

`bobe-hil net` invokes system OpenSSH with `/dev/null` config, batch mode,
strict host-key checking, the pinned `known_hosts`, the pinned identity, no
agent, and the fixed remote `/usr/sbin/bobe-netem` command. The private key
must be 0600 or stricter. The active lease this repository owns is recorded in
`private/hil/network/active-lease.json` and reconciled against the router on
every operation.

`bobe-hil net` generates and owns each lease. It persists a `pending` record
before SSH apply, promotes it from exact router status, reconciles
identity/profile/expiry before every operation, rejects foreign or overlapping
leases, and rolls back the router if active state cannot be persisted. A lost
apply response plus failed rollback therefore remains recoverable through
status or router TTL expiry.

Automated provisioning through this router is not implemented. The netem qdisc
shapes test-interface egress only; uplink loss needs bidirectional shaping that
has not been validated on hardware.
