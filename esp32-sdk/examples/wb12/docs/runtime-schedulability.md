# WB-12 runtime schedulability

BoBe uses fixed-priority FreeRTOS scheduling on both ESP32-S3 cores. The design
keeps audio deadlines on core 1, network and UI work on core 0, and every
non-periodic operation bounded or blocking below time-critical work.

| Work | Core | Priority | Period/deadline | Behavior |
|---|---:|---:|---:|---|
| I2S capture/playback | 1 | 19 | 20 ms frames | Highest application work; bounded queues |
| ESP-SR AFE | 1 | 18 | Audio-frame cadence | Preempts policy and transport |
| Core policy | 1 | 10 | Event driven | Nonblocking reducer plus bounded effects |
| Startup | 1 | 10 | Boot only | Deletes itself after initialization |
| Transport | 0 | 8 | Event driven | Bounded WebSocket/control queues |
| WebSocket | 0 | 7 | Network driven | Lower than audio and policy |
| Background/time sync | 0 | 5 | Infrequent | Yields and uses bounded waits |
| LVGL UI | 0 | 4 | 30 fps maximum | One DMA buffer, asynchronous flush |
| Provisioning | 0 | 4 | User initiated | Memory-gated before startup |
| USB onboarding | 0 | 3 | User initiated | Blocks on USB/events; cannot preempt UI |

The USB onboarding task is priority 3 and therefore cannot delay LVGL,
transport, policy, AFE, or I2S tasks through CPU scheduling. Its 6,144-byte
stack matches the provisioning task class and covers the parser, two Wi-Fi
configurations, and ESP-IDF call depth.

## Non-CPU blocking

Wi-Fi replacement is tested using `WIFI_STORAGE_RAM`. Only a verified
association and DHCP result triggers one `WIFI_STORAGE_FLASH` commit. This
avoids two flash/cache stalls on failed attempts and preserves the previous
network across reset or power loss. The remaining single NVS commit can stall
flash-backed code briefly on both cores; it occurs only during explicit setup,
while no voice turn should be active.

USB logs and protocol responses share the ESP-IDF USB Serial/JTAG VFS writer.
The 2 KiB TX ring and whole-write lock prevent response/log byte interleaving.
The onboarding reader waits 250 ms between input opportunities and then blocks
on an event group, so it does not poll or consume idle CPU.

## Runtime invariants

1. Audio and AFE always outrank policy, transport, UI, provisioning, and USB.
2. USB Wi-Fi mutation has one mutex owner and one active transaction.
3. Normal reconnect timers and provisioning do not run during a USB trial.
4. Success requires the associated SSID to match the proposal plus `GOT_IP`.
5. Failure restores the previous runtime configuration and recovery path.
6. Protocol lines and credentials are bounded; ordinary logs are ignored.
7. Display and audio health counters are the live smoothness indicators.
