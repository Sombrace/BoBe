#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if (( $# < 2 )); then
    echo "usage: $0 OUTPUT_DIRECTORY DEVICE_ID [HUB_HOST] [HUB_PORT]" >&2
    exit 2
fi
OUTPUT_REQUESTED="${1:-$ROOT/private/bodylink-pki}"
DEVICE_ID_REQUESTED="$2"
HUB_HOST_REQUESTED="${3:-bobe-body-hub.local}"
HUB_PORT="${4:-8767}"

if [[ ! "$HUB_PORT" =~ ^[0-9]+$ ]] ||
   (( HUB_PORT < 1 || HUB_PORT > 65535 )); then
    echo "hub port must be an integer from 1 through 65535" >&2
    exit 2
fi

cd "$ROOT"
VALIDATED_INPUT="$(
    uv run --locked python - \
        "$OUTPUT_REQUESTED" \
        "$DEVICE_ID_REQUESTED" \
        "$HUB_HOST_REQUESTED" \
        "$ROOT" <<'PY'
import ipaddress
import pathlib
import re
import sys
import uuid

requested, device_id, host, root_raw = sys.argv[1:]
if str(uuid.UUID(device_id)) != device_id:
    raise SystemExit("device id must use canonical lowercase UUID formatting")

try:
    address = ipaddress.ip_address(host)
except ValueError:
    if len(host) > 253:
        raise SystemExit("hub host is too long")
    labels = host.rstrip(".").split(".")
    if any(
        not label
        or len(label) > 63
        or re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?", label)
        is None
        for label in labels
    ):
        raise SystemExit("hub host must be an IP address or valid DNS hostname")
    host = host.rstrip(".").lower()
    san = f"DNS:{host}"
    endpoint_host = host
    mdns_enabled = "true"
    bind_host = "0.0.0.0"
else:
    host = str(address)
    san = f"IP:{host}"
    endpoint_host = f"[{host}]" if address.version == 6 else host
    mdns_enabled = "false"
    bind_host = "::" if address.version == 6 else "0.0.0.0"

root = pathlib.Path(root_raw).resolve()
private = root / "private"
if private.is_symlink():
    raise SystemExit("private output root must not be a symbolic link")
requested_path = pathlib.Path(requested)
output = (
    requested_path if requested_path.is_absolute() else root / requested_path
).resolve()
private_resolved = private.resolve()
if output != private_resolved and private_resolved not in output.parents:
    raise SystemExit("PKI output must be under the project private directory")
if "|" in str(output) or any(
    ord(character) < 0x20 for character in str(output)
):
    raise SystemExit("PKI output path contains control characters")

print(
    "|".join(
        (
            str(output),
            device_id,
            host,
            san,
            endpoint_host,
            mdns_enabled,
            bind_host,
        )
    )
)
PY
)"
IFS='|' read -r OUTPUT DEVICE_ID HUB_HOST PRIMARY_SAN ENDPOINT_HOST \
    MDNS_ENABLED BIND_HOST <<<"$VALIDATED_INPUT"

if [[ -L "$OUTPUT" ]]; then
    echo "PKI output must not be a symbolic link" >&2
    exit 2
fi
if [[ -d "$OUTPUT" ]] &&
   find "$OUTPUT" -mindepth 1 -print -quit | grep -q .; then
    echo "PKI output must be a new or empty directory: $OUTPUT" >&2
    exit 2
fi

umask 077
mkdir -p "$ROOT/private" "$OUTPUT"
chmod 700 "$ROOT/private" "$OUTPUT"

openssl genpkey -algorithm EC \
    -pkeyopt ec_paramgen_curve:P-256 \
    -out "$OUTPUT/ca-key.pem"
openssl req -x509 -new -sha256 -days 3650 \
    -key "$OUTPUT/ca-key.pem" \
    -subj "/CN=BoBe BodyLink Development CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -addext "subjectKeyIdentifier=hash" \
    -out "$OUTPUT/ca-cert.pem"

openssl genpkey -algorithm EC \
    -pkeyopt ec_paramgen_curve:P-256 \
    -out "$OUTPUT/server-key.pem"
openssl req -new \
    -key "$OUTPUT/server-key.pem" \
    -subj "/CN=$HUB_HOST" \
    -out "$OUTPUT/server.csr"
printf '%s\n' \
    "basicConstraints=critical,CA:FALSE" \
    "keyUsage=critical,digitalSignature,keyAgreement" \
    "extendedKeyUsage=serverAuth" \
    "subjectAltName=$PRIMARY_SAN,DNS:localhost,IP:127.0.0.1" \
    >"$OUTPUT/server.ext"
openssl x509 -req -sha256 -days 825 \
    -in "$OUTPUT/server.csr" \
    -CA "$OUTPUT/ca-cert.pem" \
    -CAkey "$OUTPUT/ca-key.pem" \
    -set_serial "0x$(openssl rand -hex 16)" \
    -extfile "$OUTPUT/server.ext" \
    -out "$OUTPUT/server-cert.pem"

openssl genpkey -algorithm EC \
    -pkeyopt ec_paramgen_curve:P-256 \
    -out "$OUTPUT/device-key.pem"
openssl req -new \
    -key "$OUTPUT/device-key.pem" \
    -subj "/CN=$DEVICE_ID" \
    -out "$OUTPUT/device.csr"
printf '%s\n' \
    "basicConstraints=critical,CA:FALSE" \
    "keyUsage=critical,digitalSignature,keyAgreement" \
    "extendedKeyUsage=clientAuth" \
    "subjectAltName=URI:urn:bobe:body:$DEVICE_ID" \
    >"$OUTPUT/device.ext"
openssl x509 -req -sha256 -days 825 \
    -in "$OUTPUT/device.csr" \
    -CA "$OUTPUT/ca-cert.pem" \
    -CAkey "$OUTPUT/ca-key.pem" \
    -set_serial "0x$(openssl rand -hex 16)" \
    -extfile "$OUTPUT/device.ext" \
    -out "$OUTPUT/device-cert.pem"

KID="$(
    openssl x509 -in "$OUTPUT/ca-cert.pem" -outform DER |
        shasum -a 256 |
        awk '{print substr($1, 1, 16)}'
)"
ADAPTER_TOKEN="$(openssl rand -hex 32)"

uv run --locked python - \
    "$OUTPUT" \
    "$DEVICE_ID" \
    "$HUB_HOST" \
    "$HUB_PORT" \
    "$KID" \
    "$ENDPOINT_HOST" \
    "$BIND_HOST" \
    "$MDNS_ENABLED" \
    "$ADAPTER_TOKEN" <<'PY'
import json
import pathlib
import shlex
import sys

output = pathlib.Path(sys.argv[1])
(
    device_id,
    host,
    port,
    kid,
    endpoint_host,
    bind_host,
    mdns_enabled,
    adapter_token,
) = sys.argv[2:]
claim = {
    "device_id": device_id,
    "endpoint_uri": f"wss://{endpoint_host}:{port}/body/v1",
    "trust_kid": kid,
    "controller_epoch": 1,
    "server_ca_pem": (output / "ca-cert.pem").read_text(),
    "client_cert_pem": (output / "device-cert.pem").read_text(),
    "client_key_pem": (output / "device-key.pem").read_text(),
}
with (output / "claim.json").open("x", encoding="utf-8") as file:
    json.dump(claim, file, indent=2)
    file.write("\n")

values = {
    "BOBE_BODY__ENABLED": "true",
    "BOBE_BODY__HOST": bind_host,
    "BOBE_BODY__ADVERTISED_HOSTNAME": host,
    "BOBE_BODY__PORT": port,
    "BOBE_BODY__ADAPTER_PORT": "8768",
    "BOBE_BODY__ADAPTER_TOKEN": adapter_token,
    "BOBE_BODY__TLS_CERT_PATH": str(output / "server-cert.pem"),
    "BOBE_BODY__TLS_KEY_PATH": str(output / "server-key.pem"),
    "BOBE_BODY__CLIENT_CA_PATH": str(output / "ca-cert.pem"),
    "BOBE_BODY__TRUST_KEY_ID": kid,
    "BOBE_BODY__CONTROLLER_EPOCH": "1",
    "BOBE_BODY__MDNS_ENABLED": mdns_enabled,
    "BOBE_BODY_ADAPTER": "1",
    "BOBE_BODY_ADAPTER_URL": "http://127.0.0.1:8768",
}
with (output / "bobe-body.env").open("x", encoding="utf-8") as file:
    for name, value in values.items():
        file.write(f"export {name}={shlex.quote(value)}\n")
PY

rm -f "$OUTPUT/server.csr" "$OUTPUT/server.ext" \
    "$OUTPUT/device.csr" "$OUTPUT/device.ext"
chmod -R go-rwx "$OUTPUT"

DEVICE_CERT_SHA256="$(
    openssl x509 -in "$OUTPUT/device-cert.pem" -outform DER |
        shasum -a 256 |
        awk '{print $1}'
)"

printf 'Generated BodyLink development PKI under %s\n' "$OUTPUT"
printf 'Trust key id: %s\n' "$KID"
printf 'Source %s to run the daemon and Swift speech adapter, then enroll the device:\n' \
    "$OUTPUT/bobe-body.env"
printf '  bobe endpoint enroll %s --kind physical_body --label WB-12 \\\n' "$DEVICE_ID"
printf '    --certificate-sha256 %s --protocol-min 1 --protocol-max 1\n' \
    "$DEVICE_CERT_SHA256"
