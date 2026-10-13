# Electrical boundary

The FX2 chip and a compatible driver profile do not establish an assembled
clone's input protection.

Until a trustworthy manual or PCB inspection proves otherwise:

- use only 0-3.3 V logic;
- power the analyzer before connecting signals;
- connect ground before signal leads;
- disconnect signals before removing analyzer power;
- do not connect negative, RS-232, mains, automotive, analog, or high-energy
  signals;
- do not assume reliable 1.8 V thresholds or 5 V tolerance.

Use level translation or hardware with a verified rating when the signal is
outside those assumptions.

The sigrok wrapper is instrument authority, not permission to open or probe a
target. The sealed WB-12 has no approved internal probe access.
