# WonderBoy WB-12 voice body

Turns the WonderBoy WB-12 AI Buddy, an ESP32-S3R8 toy with 8 MB PSRAM and 16 MB
flash, into a BoBe voice body. Pins and drivers are in
[`components/bobe_board_port`](components/bobe_board_port); [HIL tools](hil/README.md)
test a real unit. Run the commands below from `examples/wb12`.

> [!WARNING]
> BoBe is not affiliated with the WB-12's maker. Flashing replaces the factory
> application and may void the warranty. Make and verify a full flash backup
> first. Never burn eFuses, and never erase or overwrite eFuse Key0: it holds the
> vendor's read-protected `HMAC_UP` activation key. Restoring factory firmware is
> not part of this example.

| Control | Use |
|---|---|
| Main button (GPIO0) | Hold to talk; also the ROM download strap |
| Volume down (GPIO39), volume up (GPIO40) | Volume. A long press of volume down alone mutes or unmutes the microphone; mute survives reboots |
| Volume down and main, long press | Shut down |
| Both volume buttons, long press | Open the provisioning window |
| Rear button | Hardware reset |

The battery percentage uses the factory's uncalibrated curve.

## Build

```sh
tools/bootstrap_idf.sh
tools/check.sh
```

`bootstrap_idf.sh` installs ESP-IDF 6.0.2 into the SDK's `.deps/`. `check.sh`
runs every host test and profile build on macOS without flashing; pass
`core-policy`, `ui`, `fixture` or `capture` to run one part, or `ui-regenerate`
to rewrite the UI reference hashes.

| Profile | Defaults files | Adds |
|---|---|---|
| `fallback` | `sdkconfig.defaults` | Base firmware without provisioning |
| `ptt` | + `sdkconfig.provisioning.defaults` | Security 2 SoftAP provisioning |
| `edge` | + `sdkconfig.provisioning.defaults;sdkconfig.edge.defaults` | ESP-SR audio front end and optional wake word |
| `ble` | + `sdkconfig.ble.defaults` | BLE provisioning, only on hardware with a verified BLE radio |

`tools/idf.sh build` builds `fallback`; build others in their own directory:

```sh
tools/idf.sh -B build-edge -D SDKCONFIG=build-edge/sdkconfig \
  -D "SDKCONFIG_DEFAULTS=sdkconfig.defaults;sdkconfig.provisioning.defaults;sdkconfig.edge.defaults" build
```

Raise `CONFIG_BOBE_BUILD_NUMBER` for every release. Images must fit the 5 MiB
OTA slots in [`partitions.csv`](partitions.csv) and keep the log lines the HIL
tools read: `device=… boot=… firmware=…`, `health uptime=…` and
`physical button event_id=…`. The wake word needs a WakeNet model named with
`bobe` in the `model` partition; none is provided.

## Flash

```sh
tools/build_bobe_snapshot.sh --profile edge
tools/bobe_hil.sh device flash --image build-bobe-snapshot/bobe_wb12.bin \
  --rollback-image LAST_GOOD/bobe_wb12.bin
```

The snapshot comes from the committed source with a manifest and
`bobe-snapshot.lock`; [`hil/last-known-good.lock`](hil/last-known-good.lock)
records the last reviewed build. `device flash` writes and verifies `ota_0` on a
unit that already uses `partitions.csv`, leaves the bootloader, partition table,
NVS and eFuses alone, and restores the rollback image if the write or boot
fails. Never use it on a unit with Secure Boot or flash encryption; a plain
write can brick it. `tools/flash_app_verified.sh PORT IMAGE` flashes only an
image whose manifest is clean and signed for
[`manifest-signing-public.pem`](hil/security/manifest-signing-public.pem);
`tools/bobe_hil.sh fixture enter-loader` reaches the ROM loader when a USB reset
cannot.

## Provision and enroll

Keep every identity and credential in the ignored `private/` directory; never
commit or log them. Take the device ID from the `device=` line of the serial log
(`tools/idf.sh -p PORT monitor`):

```sh
tools/generate_bodylink_pki.sh private/bodylink-pki DEVICE_ID [HUB_HOST] [HUB_PORT]
tools/idf.sh reconfigure
tools/generate_provisioning_credentials.sh --create --device-id DEVICE_ID
```

These create a development CA, certificates for the daemon (default
`bobe-body-hub.local:8767`) and the device, `claim.json`, the daemon settings
`bobe-body.env` and the Security 2 credentials the build compiles in, and print
the enrollment command; `--rotate` replaces the Security 2 credentials. Build and
flash `ptt` or `edge`, start the daemon with BodyLink enabled, and enroll the
body from a second shell:

```sh
source private/bodylink-pki/bobe-body.env && bobe serve
```

```sh
bobe endpoint enroll DEVICE_ID --kind physical_body --label WB-12 \
  --certificate-sha256 SHA256 --protocol-min 1 --protocol-max 1
```

`bobe-body.env` listens on every interface but admits only enrolled
certificates; a daemon running as a service reads the same `BOBE_BODY__*`
settings from its environment or the `[body]` section of `config.toml`. A body
without Wi-Fi opens its provisioning window at boot. Join `BOBE_XXXXXX` with
`softap_password` from `private/provisioning/security2-private.json`, then send
the claim and Wi-Fi:

```sh
tools/provision_bobe.sh --ssid "HOME_WIFI" --passphrase "WIFI_PASSWORD"
```

A new claim needs a higher `controller_epoch`, and the daemon's
`BOBE_BODY__CONTROLLER_EPOCH` must equal the body's. To change Wi-Fi over USB-C,
put `BOBE_WIFI_SSID` and `BOBE_WIFI_PASSWORD` in `private/dev-wifi.env` and run
`uv run --group hil --locked python tools/configure_bobe_usb.py`; the body keeps
its old network if the new one fails. `tools/build_dev_wifi.sh` bakes the
password into an image instead: flash it only to seed Wi-Fi, then flash a normal
image without erasing NVS.

## Firmware rules

- `bobe_core` decides everything through the pure reducer `bobe_policy`; task
  cores and priorities are in
  [`bobe_task_config.h`](../../components/bobe_platform/include/bobe_task_config.h).
- Only the LVGL task calls LVGL. I2S and GPIO callbacks never touch the network.
  Audio sinks must not block or call back into `bobe_audio` or
  `bobe_voice_frontend`. A full queue ends the session.
- Keep tickless idle, light-sleep CPU power-down and Wi-Fi power saving off; they
  break USB, audio and DMA timing.
- Without flash encryption, anyone holding the body can read its claim key from
  NVS.

## Signed updates

```sh
uv run --locked python tools/create_update_offer.py \
  --image build-bobe-snapshot/bobe_wb12.bin \
  --artifact-manifest build-bobe-snapshot/bobe_wb12.bin.manifest.json \
  --private-key private/update-signing-key.pem --output private/update-offer.json \
  --url https://updates.example/bobe_wb12.bin --key-id KEY_ID \
  --current-build-number 2 --build-number 3 \
  --not-before UNIX_TIME --expires-at UNIX_TIME
```

The offer needs a clean snapshot build, and `--build-number` must equal the
image's `CONFIG_BOBE_BUILD_NUMBER`, which the manifest records. Keep the signing
key in `private/` and point the daemon at the offer with
`BOBE_BODY__UPDATE_OFFER_PATH`. The body verifies offers with
[`update-signing-public.pem`](components/bobe_board_port/update-signing-public.pem)
and downloads only over HTTPS from a server whose certificate chains to its
claim's CA ([checks](../../docs/bodylink.md#signed-updates)).

> [!WARNING]
> [`sdkconfig.release.defaults`](sdkconfig.release.defaults) adds NVS encryption
> keyed by the existing `HMAC_UP` Key0. Use it only on a unit whose Key0 already
> holds that key; on another chip ESP-IDF may burn a new key at first boot. It
> does not convert plaintext NVS, so migrate the body's Wi-Fi, claim and identity
> deliberately.
>
> Secure Boot v2 and flash encryption are irreversible eFuse changes that no tool
> here makes. Rehearse on a unit you can lose, keep the keys offline, and never
> recover an enrolled unit with a plaintext factory image.
