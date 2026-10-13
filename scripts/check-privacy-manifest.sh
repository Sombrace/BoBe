#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MANIFEST="${1:-$ROOT/BoBeMacUI/BoBe/Resources/PrivacyInfo.xcprivacy}"

[[ -f "$MANIFEST" ]] || {
    echo "Privacy manifest not found: $MANIFEST" >&2
    exit 1
}

if [[ "$(uname -s)" == Darwin ]]; then
    plutil -lint "$MANIFEST" >/dev/null
fi

python3 - "$MANIFEST" <<'PY'
import pathlib
import plistlib
import sys

path = pathlib.Path(sys.argv[1])
with path.open("rb") as source:
    manifest = plistlib.load(source)

if manifest.get("NSPrivacyTracking") is not False:
    raise SystemExit(f"{path}: NSPrivacyTracking must be false")
if manifest.get("NSPrivacyTrackingDomains") != []:
    raise SystemExit(f"{path}: tracking domains must be empty")

accessed = manifest.get("NSPrivacyAccessedAPITypes")
expected_accessed = [
    {
        "NSPrivacyAccessedAPIType": "NSPrivacyAccessedAPICategoryUserDefaults",
        "NSPrivacyAccessedAPITypeReasons": ["CA92.1"],
    }
]
if accessed != expected_accessed:
    raise SystemExit(
        f"{path}: required-reason APIs must be exactly UserDefaults/CA92.1; "
        f"found {accessed!r}"
    )

collected = manifest.get("NSPrivacyCollectedDataTypes")
expected_purposes = {
    "NSPrivacyCollectedDataTypeOtherUserContent": [
        "NSPrivacyCollectedDataTypePurposeAppFunctionality",
        "NSPrivacyCollectedDataTypePurposeProductPersonalization",
    ],
    "NSPrivacyCollectedDataTypePhotosorVideos": [
        "NSPrivacyCollectedDataTypePurposeAppFunctionality",
        "NSPrivacyCollectedDataTypePurposeProductPersonalization",
    ],
    "NSPrivacyCollectedDataTypeUserID": [
        "NSPrivacyCollectedDataTypePurposeAppFunctionality",
    ],
}
expected_types = set(expected_purposes)
if not isinstance(collected, list):
    raise SystemExit(f"{path}: collected-data declarations must be an array")
actual_types = {item.get("NSPrivacyCollectedDataType") for item in collected}
if actual_types != expected_types or len(collected) != len(expected_types):
    raise SystemExit(
        f"{path}: collected-data types differ from the audited cloud/remote flows; "
        f"found {sorted(str(value) for value in actual_types)}"
    )
expected_fields = {
    "NSPrivacyCollectedDataType",
    "NSPrivacyCollectedDataTypeLinked",
    "NSPrivacyCollectedDataTypeTracking",
    "NSPrivacyCollectedDataTypePurposes",
}
for item in collected:
    data_type = item["NSPrivacyCollectedDataType"]
    if set(item) != expected_fields:
        raise SystemExit(f"{path}: {data_type} contains unexpected or missing fields")
    if item.get("NSPrivacyCollectedDataTypeLinked") is not True:
        raise SystemExit(f"{path}: {data_type} must remain declared as linked")
    if item.get("NSPrivacyCollectedDataTypeTracking") is not False:
        raise SystemExit(f"{path}: {data_type} must not be declared for tracking")
    purposes = item.get("NSPrivacyCollectedDataTypePurposes")
    if purposes != expected_purposes[data_type]:
        raise SystemExit(
            f"{path}: {data_type} purposes differ from the audited contract; "
            f"found {purposes!r}"
        )

print(f"Privacy manifest is valid: {path}")
PY
