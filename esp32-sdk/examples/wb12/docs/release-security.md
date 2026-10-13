# WB-12 release security preparation

This file documents the irreversible release transition. It is not executed by
`tools/check.sh`, normal builds, or flashing commands.

## Starting state

- `BLOCK_KEY0` is read-protected with purpose `HMAC_UP`, as the factory
  provisions it.
- Secure Boot is disabled.
- Flash encryption is disabled.
- ROM download and USB Serial/JTAG recovery are available.
- You have a complete backup of the flash and the readable eFuse state.

## Reversible release candidate

`../sdkconfig.release.defaults` enables NVS encryption using the existing
`HMAC_UP` Key0. Build it in a separate directory together with the normal and
edge defaults. Do not apply it to another ESP32-S3 unless its selected key block
is already provisioned for `HMAC_UP`; ESP-IDF may otherwise generate and burn a
new HMAC key during first boot.

The plaintext default NVS partition must be migrated deliberately. Enabling NVS
encryption against an existing plaintext partition is not an in-place format
conversion. Export required Wi-Fi, claim, TLS, identity, settings, and clock
state; initialize encrypted NVS; then restore only validated records.

## Signed OTA offer creation

Private update keys remain under `private/`. Create a clean firmware artifact
manifest first, then generate the exact canonical BodyLink envelope:

```bash
uv run --locked python tools/create_update_offer.py \
  --image build-release/bobe_wb12.bin \
  --artifact-manifest build-release/bobe_wb12.bin.manifest.json \
  --private-key private/update-signing-key.pem \
  --output private/update-offer-0.3.0.json \
  --url https://updates.example/bobe_wb12-0.3.0.bin \
  --key-id prod-2026-01 \
  --current-build-number 2 \
  --build-number 3 \
  --not-before 1780000000 \
  --expires-at 1780604800
```

The tool refuses dirty artifacts, mismatched image hashes/sizes, downgrade build
numbers, non-HTTPS URLs, unsafe paths, and existing output files. It signs the
canonical manifest bytes with ECDSA P-256/SHA-256 and writes the envelope mode
600. The daemon consumes only this pre-signed envelope; it never receives the
private key.

## Irreversible production enrollment

Secure Boot v2 and flash encryption require a separate ceremony:

1. Generate production signing and flash-encryption keys in offline custody.
2. Build and verify signed bootloader, partition table, and both OTA slots.
3. Record full eFuse preflight and confirm free key blocks/purposes.
4. Verify encrypted-NVS migration and signed OTA on a sacrificial unit first.
5. Burn flash-encryption and Secure Boot keys/digests.
6. Reboot and verify encrypted boot, signature enforcement, disabled plaintext
   reads, rollback, signed OTA, and the chosen ROM-download/JTAG policy.
7. Archive the post-enrollment eFuse summary and recovery instructions.

Do not use a pre-enrollment plaintext factory image for recovery after these
eFuses are burned. Use a recovery procedure that matches the device's enrolled
keys and security policy.
