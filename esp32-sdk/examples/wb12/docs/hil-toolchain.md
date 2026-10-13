# Autonomous validation toolchain

Tools are selected for concrete WB-12 coverage, maintenance authority and
project-local installation.

## Adopted

| Tool | Pinned version | Authority and role |
|---|---:|---|
| [ESP-IDF](https://github.com/espressif/esp-idf/releases/tag/v6.0.2) | 6.0.2 | Espressif SDK, build, USB/JTAG support and target runtime |
| [esptool](https://github.com/espressif/esptool/releases/tag/v5.4.0) | 5.4.0 | Espressif ROM/image transport behind the project safety wrapper |
| Python `unittest` | standard library | Dynamically discovered host tests without an auto-flash plugin |
| [LVGL](https://github.com/lvgl/lvgl/releases/tag/v9.5.0) | 9.5.0 | Shared production renderer and canonical RGB565 visual oracle |
| [Raspberry Pi Pico SDK](https://github.com/raspberrypi/pico-sdk/releases/tag/2.3.0) | 2.3.0 | Official Pico 2 fixture firmware SDK |
| [Pico SDK tools](https://github.com/raspberrypi/pico-sdk-tools/releases/tag/v2.3.0-0) | 2.3.0-0 | Official macOS RISC-V toolchain/picotool/OpenOCD bundles |
| [OpenWrt](https://downloads.openwrt.org/releases/25.12.5/) | 25.12.5 | Isolated 2.4 GHz test LAN and native `tc`/`nft` fault injection |
| Apple AVFoundation/Vision/CoreImage | Xcode 26.6 SDK | Exact-device camera capture, OCR and perspective correction |
| Pillow | 12.3.0 | Project-local decoded-image metrics and conversions |
| NumPy | 2.4.2 | Python 3.11-compatible deterministic image/audio analysis |
| sounddevice | 0.5.5 | Project-local CoreAudio discovery, bounded capture, and playback |
| websockets | 16.1.1 | Deterministic BodyLink mTLS/WebSocket peer |

The Python lab packages live in a separate `hil` uv dependency group. ESP-IDF
and Pico toolchains live under ignored `.deps/`. Apple frameworks come from the
system SDK.

Every tool here validates the normal BoBe product firmware from outside the
device; see [`hil/README.md`](../hil/README.md). USB serial also carries the
product's bounded Wi-Fi onboarding protocol; it does not provide a general
control shell.

Hardware access runs in the same project environment: `/dev/cu.*` on macOS is
world read/write, so `tools/bobe_hil.sh` needs no privilege.
[Snapshot builds](../tools/README.md) write their provenance lock to
`build-bobe-snapshot/bobe-snapshot.lock`. The tracked `hil/bobe-snapshot.lock`
records an earlier build; new builds do not overwrite it.

## Planned independent observer

Physical acoustic evidence is planned to come from one physically trusted
Raspberry Pi observer instead of the macOS user session.
[`hil/observer/README.md`](../hil/observer/README.md) gives its threat model and
platform tools. OpenWrt is a separate network-fault authority.

Pi OS and package versions are chosen when the hardware is selected. The exact
dated image, package inventory, deployment hash and operator-signed
qualification are then pinned together.

## Adopted references, not runtime dependencies

- [Espressif ESP Test Tools and Guidelines](https://github.com/espressif/esp-test-tools)
  documents production fixtures, serial/EN/GPIO0 contacts, GPIO conductivity,
  firmware/hash checks, RF signal boards and shielded testing for ESP32-S3.
- [ESP-IDF AppTrace](https://docs.espressif.com/projects/esp-idf/en/v6.0.2/esp32s3/api-guides/app_trace.html)
  and bundled OpenOCD/GDB are bounded post-failure diagnostic options.
- [Agent Skills](https://docs.github.com/en/copilot/concepts/agents/about-agent-skills)
  is the common format used for the repository's sigrok and macOS camera tool
  skills.

## Optional after hardware exists

| Tool | Reason |
|---|---|
| Saleae Logic 2 Automation | Mature vendor API for real SPI/I2S capture; requires genuine fast Saleae hardware |
| OpenOCD/AppTrace | Useful bounded read-only/post-mortem evidence, but intrusive and capable of unsafe writes if exposed raw |
| Wokwi CLI 0.26.1 | Official Wokwi automation/screenshots; ESP32-S3 I2S is unsupported and there is no official ST7789 part |
| Espressif QEMU | CPU/boot smoke only; lacks the WB-12 Wi-Fi, I2S, ST7789, battery and USB physical paths |
| labgrid 26.0 | Add when multiple remote fixtures need leases and Linux exporters |

## Evaluated and not adopted

- ESP-IDF's stock MCP flash tool and `idf.py flash`/monitor flash shortcuts.
  `bobe-hil device flash` writes, verifies and boot-proves against the normal
  firmware manifest and ELF digest instead.
- pytest-embedded ESP/IDF/JTAG plugins, whose DUT objects duplicate the flash
  path this repository already implements with read-back verification.
- Community serial/debugger/sigrok MCP servers. They have one or a few primary
  maintainers; that maturity does not justify the dependency.
- GreatFET, Bus Pirate, MCP2221 and FT232H as unattended button actuators. Their
  interfaces lack the watchdog and GPIO0/EN interlock the Pico 2 fixture
  implements in firmware.
- Generic USB relays without documented identity, reset state and watchdog.
- FFmpeg, OpenCV, ImageMagick, Tesseract and SoX as dependencies. They are
  mature, but the selected Apple/Pillow/NumPy subset provides the needed
  functions without global native installation.
- Wokwi/QEMU/logic captures as substitutes for physical LCD, acoustic, RF,
  battery or mechanical-button evidence.

## Version checks

```bash
tools/bobe_hil.sh --version
tools/bobe_capture.sh --version
tools/check.sh fixture
uv run --group hil --locked python -c \
  'import numpy, PIL, sounddevice, websockets; print(numpy.__version__, PIL.__version__, sounddevice.__version__, websockets.__version__)'
```

Release updates must retain hashes/licenses, rerun the complete gate, and
invalidate affected goldens or calibrations for explicit review.
