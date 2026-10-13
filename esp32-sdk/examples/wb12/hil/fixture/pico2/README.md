# Pico 2 button/reset fixture

The fixture uses a Raspberry Pi Pico 2 running the RP2350 RISC-V target from the
official Pico SDK 2.3.0. `tools/build_pico_fixture.sh` downloads only the pinned
official SDK/toolchain artifacts and produces:

```text
build-pico-fixture/bobe_fixture.uf2
```

`picotool` or UF2 installation is an operator setup/recovery action. Agents use
only the runtime USB CDC semantic protocol.

## Electrical channels

| Pico output | Toy contact | Meaning |
|---:|---|---|
| GP2 | GPIO0 switch pad | Action/PTT |
| GP3 | GPIO39 switch pad | Volume down |
| GP4 | GPIO40 switch pad | Volume up |
| GP5 | EN/reset switch pad | Hardware reset |
| GP6 | Loader gate enable | Permit GPIO0 only during the firmware loader sequence |

Each Pico output drives a separate open-drain stage; it never connects directly
to a toy signal:

- 2N7002/BSS138 drain to the switch signal and source to verified toy ground;
- 100 kOhm gate pulldown;
- 100-1,000 Ohm gate series resistor;
- common ground only after voltage/contact characterization.

Normally-open PhotoMOS contacts are preferred until the button pads and grounded
sides are confirmed with a meter.

GPIO0 requires an independent hardware gate. In normal operation it permits the
action shunt only while EN is high; an EN pulse therefore blocks accidental
strap assertion. A separate GP6 loader-enable input may override that block only
when it and the action drive are both asserted. GP6 has its own hardware
pulldown. Firmware interlocks are additional defense, not the only protection.

## Runtime protocol

The host requires the configured 16-hex board identity and firmware version
`0.3.0`, then starts a session with a 16-byte lowercase hexadecimal nonce:

```text
HELLO 1 NONCE
```

Then only these bounded commands are accepted:

```text
STATUS SEQ NONCE
PRESS SEQ NONCE ACTION|VOLUME_DOWN|VOLUME_UP 30..2000
CHORD SEQ NONCE SHUTDOWN|PROVISION_RECOVERY 30..2000
RESET SEQ NONCE 50..500
ENTER_LOADER SEQ NONCE
RELEASE_ALL SEQ NONCE
```

There is no arbitrary GPIO, indefinite DOWN, PWM, power, caller-timed
bootloader sequence, or raw
script command. Sequence replay does not repeat an actuation. A one-second watchdog, 30-second
session expiry, and external gate pulldowns return every channel to released
state. The firmware emits exactly one response per command; there are no
unsolicited event lines that can desynchronize the host.

`RESET` requires all outputs released, waits 200 ms, pulses EN, and inhibits
action for another 500 ms. Its first response means scheduled, not complete;
the host polls strict `STATUS` frames until reset and lockout both clear.

`ENTER_LOADER` is a separate bounded state machine: assert GP6 loader enable and
action/GPIO0, wait 50 ms, pulse EN for 120 ms, hold the strap for 100 ms from
actual EN release, then release both lines and apply lockout. Both lines have an
independent one-second firmware timeout plus hardware pulldowns. Its timing is
fixed in fixture firmware, so `bobe-hil fixture enter-loader` takes no duration
argument.

## Uncharacterized hardware

USB removal does not cold-cycle the battery-powered WB-12. Do not add a battery
switch until the internal power path is characterized.

## Host configuration

The official RP2350 stdio USB descriptor uses Raspberry Pi VID `0x2e8a`, PID
`0x0009`, and the uppercase 16-hex Pico unique board ID as USB serial. Copy
`fixture-config.example.json` to the gitignored repository-local path:

```text
private/hil/fixture.json
```

Set `board_id` to the lowercase ID returned by `HELLO`. The file must be a
regular non-symlink file.
