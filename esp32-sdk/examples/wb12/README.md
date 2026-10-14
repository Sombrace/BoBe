# WB-12 example for the BoBe ESP32 SDK

This example turns the WonderBoy WB-12 AI Buddy, an off-the-shelf ESP32-S3 toy,
into a physical body for BoBe. It contains the application, the board port in
[`components/bobe_board_port`](components/bobe_board_port) and the hardware
validation tooling. Shared firmware is in [`../../components`](../../components);
see the [SDK README](../../README.md) for reusable components and other boards.

BoBe is not affiliated with the WB-12's maker. Flashing replaces the factory
application and may void a warranty; you are responsible for your device and for
radio and safety rules where you use it. Keep a verified backup of your unit
before flashing, and never burn eFuses or erase the factory activation key.

- Architecture: [`docs/architecture.md`](docs/architecture.md)
- BodyLink protocol: [`docs/bodylink-v1.md`](docs/bodylink-v1.md)
- Hardware-in-the-loop validation: [`hil/README.md`](hil/README.md) and
  [`docs/hil-toolchain.md`](docs/hil-toolchain.md)
- Release security: [`docs/release-security.md`](docs/release-security.md)
- Source: [`main/`](main/) and [`components/`](components/); dependencies in
  [`bobe_sdk/idf_component.yml`](../../components/bobe_sdk/idf_component.yml) and
  [`dependencies.lock`](dependencies.lock)

Build and test without flashing:

```bash
tools/bootstrap_idf.sh
tools/check.sh
```

`tools/check.sh` never runs `flash`, `app-flash`, `erase-flash` or an eFuse
operation; [`tools/README.md`](tools/README.md) lists what it checks. Explicit
flashing replaces an application slot. Signed A/B OTA and rollback remain
supported. Restoring a unit's factory firmware is not part of this example.

## Hardware

The pin map below was read from the factory application and cross-checked against
ESP-IDF and the Xiaozhi audio and button drivers it derives from. Part numbers that
software cannot read are unknown.

| Area | Value |
|---|---|
| MCU | ESP32-S3 QFN56 rev 0.2 (ESP32-S3R8), 240 MHz, 40 MHz crystal, Wi-Fi and Bluetooth 5 LE |
| RAM | 8 MB in-package octal PSRAM, 3.3 V, 80 MHz |
| Flash | 16 MB Boya B.25Q128AS (JEDEC `68:4018`); headers request DIO 80 MHz, the runtime switches to QIO |
| Display | ST7789, 240 x 280, SPI, no touch controller |
| Audio | Separate I2S MEMS microphone and I2S DAC/class-D amplifier, no codec bus or MCLK |
| Power | Single-cell Li-ion with ADC divider, active-low charge status and a power-hold line |
| USB | Native USB Serial/JTAG over USB-C |

### GPIO map

| GPIO | Direction | Function | Detail |
|---:|---|---|---|
| 0 | Input | Main button | Active-low, pull-up; ROM download strap |
| 4 | Output | Microphone WS | I2S1 RX, master |
| 5 | Output | Microphone BCLK | I2S1 RX, master |
| 6 | Input | Microphone data | I2S1 RX |
| 7 | Output | Speaker data | I2S0 TX |
| 8 | Output | LCD D/C | |
| 9 | Output | LCD SCLK | SPI3 |
| 10 | Output | LCD MOSI | SPI3, no MISO |
| 13 | Output | LCD backlight | 25 kHz PWM, non-inverted |
| 14 | Output | LCD CS | |
| 15 | Output | Speaker BCLK | I2S0 TX, master |
| 16 | Output | Speaker WS | I2S0 TX, master |
| 17 | Analog | Battery voltage | ADC2 channel 6, divider |
| 18 | Output | LCD reset | |
| 19, 20 | USB | USB D-, D+ | Fixed native USB pins |
| 21 | Output | Power hold | Active-high; low requests shutdown |
| 38 | Input | Charge status | Active-low |
| 39 | Input | Volume down | Active-low, pull-up |
| 40 | Input | Volume up | Active-low, pull-up |

No other GPIO has a known board use.

### Display

ST7789 on SPI3, mode 3, 80 MHz, RGB565, write-only DMA. The driver is created as
280 x 240 with X and Y swapped, X offset 20, mirror X and inverted colour.

### Audio

| Path | Controller | Rate | Slot |
|---|---|---|---|
| Speaker | I2S0 TX | 24 kHz | Mono left, 16-bit data in 32-bit slots |
| Microphone | I2S1 RX | 16 kHz | Mono left, 32-bit slots |

### Buttons

GPIO0 is hold-to-talk while online and stays the ROM download strap; holding it
alone never interrupts an active voice turn. Holding volume-down with GPIO0 requests
a software shutdown. Holding both volume buttons opens provisioning recovery. A
fourth rear button is a hardware reset and produces no application event.

### Power

GPIO21 is driven high at boot to hold power; shutdown drives it low and enters deep
sleep. The factory firmware reads GPIO17 with 12-bit resolution and 12 dB
attenuation and shuts down at about 3.4 V. Its percentage table is uncalibrated:

| Raw ADC | 1970 | 2062 | 2154 | 2246 | 2338 | 2430 |
|---|---:|---:|---:|---:|---:|---:|
| Level | 0% | 20% | 40% | 60% | 80% | 100% |

### Flash layout

| Label | Type | Offset | Size |
|---|---|---:|---:|
| `nvs` | data/NVS | `0x009000` | `0x004000` |
| `otadata` | data/OTA | `0x00D000` | `0x002000` |
| `phy_init` | data/PHY | `0x00F000` | `0x001000` |
| `ota_0` | app/OTA0 | `0x020000` | `0x3F0000` |
| `ota_1` | app/OTA1 | `0x410000` | `0x3F0000` |
| `assets` | data/SPIFFS | `0x800000` | `0x800000` |

### Programming

Secure Boot and flash encryption are off, and the unit is readable and
programmable over native USB. eFuse Key0 holds the vendor's read- and
write-protected `HMAC_UP` activation key; never burn, replace or erase it.
