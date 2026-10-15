# WonderBoy WB-12 voice body

This example turns the WonderBoy WB-12 AI Buddy, an off-the-shelf ESP32-S3 toy,
into a BoBe voice body on the [SDK](../../README.md): the application in
[`main/`](main), the board port in [`components/bobe_board_port`](components/bobe_board_port),
device constants in [`components/bobe_device_config`](components/bobe_device_config)
and the [hardware-in-the-loop tools](hil/README.md). Run the commands below from
`examples/wb12`.

> [!WARNING]
> BoBe is not affiliated with the WB-12's maker. Flashing replaces the factory
> application and may void the warranty. Make and verify a full flash backup
> first. Never burn eFuses, and never erase or overwrite eFuse Key0: it holds the
> vendor's read-protected `HMAC_UP` activation key. Restoring factory firmware is
> not part of this example.

## Hardware

| Part | Detail |
|---|---|
| Chip | ESP32-S3R8: dual-core 240 MHz, 8 MB octal PSRAM, Wi-Fi |
| Flash | 16 MB |
| Display | ST7789, 240 × 280 panel driven as 280 × 240 landscape over SPI3 |
| Audio | I2S MEMS microphone on I2S1 and I2S amplifier on I2S0; no codec |
| Power | Single-cell Li-ion with a power-hold line, charge-status input and battery ADC |
| USB | Native USB Serial/JTAG on USB-C |

| Control | GPIO | Use |
|---|---|---|
| Main button | 0 | Hold to talk; also the ROM download strap |
| Volume down | 39 | Volume down; a long press alone turns microphone mute on or off |
| Volume up | 40 | Volume up |
| Volume down and main, long press | 39, 0 | Shut down |
| Both volume buttons, long press | 39, 40 | Open the provisioning window |
| Rear button | EN | Hardware reset; the firmware never sees it |

The other pins (display, microphone, speaker, power hold 21, charge status 38,
battery 17) are set in [`components/bobe_board_port`](components/bobe_board_port).
Mute is a software control and survives reboots. The battery percentage uses the
factory's uncalibrated curve. Offline, the body keeps its local controls and
never stores audio to send later.

## Build and check

```sh
tools/bootstrap_idf.sh
tools/check.sh
```

`tools/bootstrap_idf.sh` prepares ESP-IDF 6.0.2 in the SDK's `.deps/`.
`tools/check.sh` runs the host and sanitizer tests, the UI reference images, the
Pico fixture build, the macOS camera-tool tests and a build of every profile
below. The full run needs macOS and never flashes. `tools/check.sh` with
`core-policy`, `ui`, `fixture` or `capture` runs one part; `ui-regenerate`
rewrites the UI reference hashes.

| Profile | Defaults files | Adds |
|---|---|---|
| `fallback` | `sdkconfig.defaults` | Base firmware without provisioning |
| `ptt` | plus `sdkconfig.provisioning.defaults` | Security 2 SoftAP provisioning |
| `edge` | plus `sdkconfig.provisioning.defaults` and `sdkconfig.edge.defaults` | ESP-SR audio front end and optional wake word |
| `ble` | plus `sdkconfig.ble.defaults` | BLE provisioning, only for hardware with a verified BLE radio; the WB-12 uses SoftAP |

`tools/idf.sh build` builds `fallback`. Build another profile in its own
directory, as `tools/check.sh` does:

```sh
tools/idf.sh -B build-edge -D SDKCONFIG=build-edge/sdkconfig \
  -D "SDKCONFIG_DEFAULTS=sdkconfig.defaults;sdkconfig.provisioning.defaults;sdkconfig.edge.defaults" build
```

Every image must fit the 5 MiB OTA slots in [`partitions.csv`](partitions.csv)
and keep the log lines the HIL tools read: `device=… boot=… firmware=…`,
`health uptime=…` and `physical button event_id=…`. Raise
`CONFIG_BOBE_BUILD_NUMBER` for every release. The edge profile listens for a
wake word only if the `model` partition holds a WakeNet model whose name
contains `bobe`; none is provided. `dependencies.lock` and `uv.lock` pin the ESP
components and Python packages.

## Flash

Both tools below write one application image to a WB-12 that already uses
`partitions.csv`, read it back to verify it, and leave the bootloader, partition
table, NVS and eFuses alone.

```sh
tools/build_bobe_snapshot.sh --profile edge
tools/bobe_hil.sh device flash --image build-bobe-snapshot/bobe_wb12.bin \
  --rollback-image LAST_GOOD/bobe_wb12.bin
```

`build_bobe_snapshot.sh` builds `fallback`, `ptt` or `edge` from the committed
source only. It writes `build-bobe-snapshot/` with the image, its manifest and
`bobe-snapshot.lock` (source commit, image and ELF hashes), and replaces an
existing build only with `--replace`. [`hil/last-known-good.lock`](hil/last-known-good.lock)
holds the hashes of the last reviewed build for rollback.

`device flash` uses the one WB-12 on USB (or `--port`), writes `ota_0` by
default, and ties the observed reboot to the image when the manifest names its
ELF; if the write or boot fails, it restores the rollback image. It only warns
about Secure Boot or flash encryption, where a plain write can brick the unit,
so do not use it there.

`tools/flash_app_verified.sh PORT IMAGE` writes only `0x20000`. It requires an
`IMAGE.manifest.json` from a clean tree signed for
[`hil/security/manifest-signing-public.pem`](hil/security/manifest-signing-public.pem),
Secure Boot and flash encryption off, and `ota_0` selected. If the USB reset
cannot reach the ROM loader, use `tools/bobe_hil.sh fixture enter-loader`.

## Provision and enroll

Each body needs its own BodyLink identity and Security 2 credentials. Both stay
in the ignored `private/` directory; never commit or log them. Read the body's
device ID from the `device=` line of its serial log (`tools/idf.sh -p PORT
monitor`), then:

```sh
tools/generate_bodylink_pki.sh private/bodylink-pki DEVICE_ID [HUB_HOST] [HUB_PORT]
tools/idf.sh reconfigure
tools/generate_provisioning_credentials.sh --create --device-id DEVICE_ID
```

The PKI script creates a development CA, the daemon's certificate for `HUB_HOST`
(default `bobe-body-hub.local`, port 8767), the device certificate and key,
`claim.json` and the daemon settings `bobe-body.env`, and prints the enrollment
command. The credentials script writes the Security 2 verifier and the SoftAP
password to `private/provisioning/`, which the build compiles in; the SRP
password stays in `security2-private.json`. `--rotate` archives the bundle and
makes a new one; nothing rotates on its own.

Build and flash the `ptt` or `edge` profile. BodyLink is off by default; the
daemon reads `BOBE_BODY__*` settings from its environment or the `[body]`
section of `config.toml`. With no background service running:

```sh
source private/bodylink-pki/bobe-body.env && bobe serve
bobe endpoint enroll DEVICE_ID --kind physical_body --label WB-12 \
  --certificate-sha256 SHA256 --protocol-min 1 --protocol-max 1   # second shell
```

`bobe-body.env` binds the listener on every interface; it admits only enrolled
client certificates. A body without Wi-Fi opens its provisioning window at boot;
a set-up body opens it after a long press of both volume buttons. Join the
body's network `BOBE_XXXXXX` with `softap_password` from
`private/provisioning/security2-private.json`, then send the claim and Wi-Fi:

```sh
tools/provision_bobe.sh --ssid "HOME_WIFI" --passphrase "WIFI_PASSWORD"
```

Each completed phase is recorded in `private/provisioning-receipts/`. A new claim
for a provisioned body needs a higher `controller_epoch` than its current one,
and the daemon's `BOBE_BODY__CONTROLLER_EPOCH` must equal the body's claim.

To change Wi-Fi over USB-C instead, put `BOBE_WIFI_SSID` and `BOBE_WIFI_PASSWORD`
in `private/dev-wifi.env` and run
`uv run --group hil --locked python tools/configure_bobe_usb.py`. It sends them
only to a device that identifies as a WB-12 body, and the body keeps its previous
network if the new one fails. `tools/build_dev_wifi.sh` compiles the same Wi-Fi
into an image in `build-dev-wifi/`, which then contains the password: flash it
only to seed Wi-Fi, then flash a normal image over it without erasing NVS.

## Firmware rules

| Component | Owns |
|---|---|
| `bobe_board_port` (here) | Pins, power hold, buttons, battery, backlight, panel and I2S setup, public update key |
| `bobe_core` | Lifecycle and turn state; decisions go through `bobe_policy`, a pure reducer the host tests run unchanged |
| `bobe_transport`, `bobe_protocol` | The BodyLink connection, its bounded queues and the wire format |
| `bobe_audio`, `bobe_voice_frontend` | I2S capture and playback; optional ESP-SR front end |
| `bobe_display` | The LVGL face and themes |
| `bobe_connectivity`, `bobe_provisioning`, `bobe_usb_onboarding` | Wi-Fi, mDNS, SNTP, Security 2 provisioning, Wi-Fi setup over USB |
| `bobe_update`, `bobe_platform` | Update download and install; device ID, claim storage and task settings |

- Task cores and priorities are in
  [`bobe_task_config.h`](../../components/bobe_platform/include/bobe_task_config.h).
  Audio and the ESP-SR front end run on core 1 above policy, transport, UI,
  provisioning and USB setup.
- Only the LVGL task calls LVGL. I2S and GPIO callbacks never touch the network.
  Audio and voice front-end sinks run on worker tasks and must not block or call
  back into `bobe_audio` or `bobe_voice_frontend`. Queues are bounded; an
  overflow ends the session.
- `tools/check.sh` rejects tickless idle and light-sleep CPU power-down, which
  break USB, audio and DMA timing. Wi-Fi power saving stays off because modem
  sleep causes speaker underruns.
- The claim's private key is stored in NVS. Without flash encryption, anyone
  holding the body can read it.

## Updates and release security

Create a signed offer from a clean snapshot build. Keep the signing key under
`private/`; the daemon only ever holds the signed envelope.

```sh
uv run --locked python tools/create_update_offer.py \
  --image build-bobe-snapshot/bobe_wb12.bin \
  --artifact-manifest build-bobe-snapshot/bobe_wb12.bin.manifest.json \
  --private-key private/update-signing-key.pem --output private/update-offer.json \
  --url https://updates.example/bobe_wb12.bin --key-id KEY_ID \
  --current-build-number 2 --build-number 3 \
  --not-before UNIX_TIME --expires-at UNIX_TIME
```

`--build-number` must equal the image's `CONFIG_BOBE_BUILD_NUMBER`; otherwise
the body is offered the same update again after installing it. The body checks
offers against [`update-signing-public.pem`](components/bobe_board_port/update-signing-public.pem)
and downloads only over HTTPS from a server whose certificate chains to the CA
in its claim. Point the daemon at the envelope with
`BOBE_BODY__UPDATE_OFFER_PATH`. [BodyLink](../../docs/bodylink.md#signed-updates)
describes the offer and the body's checks.

[`sdkconfig.release.defaults`](sdkconfig.release.defaults) adds NVS encryption
keyed by the existing `HMAC_UP` Key0. Use it only on a unit whose Key0 already
holds that key; on another chip ESP-IDF may burn a new key at first boot. It
does not convert plaintext NVS, so the body's Wi-Fi, claim and identity must be
migrated deliberately.

Secure Boot v2 and flash encryption are irreversible eFuse changes that no tool
here makes. Rehearse on a unit you can lose, keep the keys offline, and never
recover an enrolled unit with a plaintext factory image.
