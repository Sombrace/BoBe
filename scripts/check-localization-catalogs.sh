#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/lib/locales.sh"
MAC_SOURCE_ROOT="$ROOT/BoBeMacUI/BoBe"
MAC_CATALOG_ROOT="$MAC_SOURCE_ROOT/Resources/i18n"
PHONE_SOURCE_ROOT="$ROOT/BoBeiOS/BoBe"
PHONE_CATALOG_ROOT="$PHONE_SOURCE_ROOT/Resources/i18n"
PHONE_SHARED_SOURCES=("$MAC_SOURCE_ROOT/Services/PairingExchange.swift")
TEMP_DIR="$ROOT/build/localization-check-$$"
mkdir -p "$ROOT/build"
mkdir -m 0700 "$TEMP_DIR"
trap 'rm -rf "$TEMP_DIR"' EXIT

matches() {
    rg "$@" || [[ $? -eq 1 ]]
}

extract_keys() {
    sed -nE 's/^[[:space:]]*"([^"]+)"[[:space:]]*=.*/\1/p' "$1"
}

check_catalogs() {
    local english="$1"
    local work="$2"
    local table
    table="$(basename "$english")"
    local catalog_root
    catalog_root="$(dirname "$(dirname "$english")")"
    mkdir "$work"

    if [[ "$(uname -s)" == Darwin ]]; then
        local locale
        for locale in "${BOBE_LOCALES[@]}"; do
            if [[ -f "$catalog_root/$locale.lproj/$table" ]]; then
                plutil -lint "$catalog_root/$locale.lproj/$table" >/dev/null
            fi
        done
    fi

    python3 - "$english" "$catalog_root" "${BOBE_LOCALES[@]}" <<'PY'
import pathlib
import re
import sys

english_path = pathlib.Path(sys.argv[1])
catalog_root = pathlib.Path(sys.argv[2])
locales = sys.argv[3:]
entry = re.compile(r'^"((?:[^"\\]|\\.)+)" = "((?:[^"\\]|\\.)*)";$')
conversion = re.compile(
    r'%(?!%)(?:(\d+)\$)?[-+ #0]*(?:\d+)?(?:\.(?:\d+))?'
    r'((?:hh|h|ll|l|L|z|j|t|q)?[@diuoxXfFeEgGaAcCsSp])'
)


def catalog_values(path):
    values = {}
    in_comment = False
    for number, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.strip()
        if in_comment:
            if "*/" in line:
                in_comment = False
            continue
        if line.startswith("/*"):
            in_comment = "*/" not in line
            continue
        if not line or line.startswith("//"):
            continue
        match = entry.fullmatch(line)
        if match is None:
            raise SystemExit(f"{path}:{number}: invalid strings entry")
        key, value = match.groups()
        if key in values:
            raise SystemExit(f"{path}:{number}: duplicate localization key {key!r}")
        values[key] = value
    return values


def signature(value):
    sequential_position = 0
    result = []
    for match in conversion.finditer(value):
        if match.group(1) is None:
            sequential_position += 1
            position = sequential_position
        else:
            position = int(match.group(1))
        result.append((position, match.group(2)))
    return sorted(result)


def report(catalog, problem, keys):
    return f"{catalog} {problem}:\n" + "\n".join(f"  {key}" for key in keys)


if english_path.parent != catalog_root / "en.lproj" or "en" not in locales:
    raise SystemExit(f"{english_path} must be the en.lproj source catalog of the canonical locales")
english = catalog_values(english_path)
problems = []
present = {path.name[: -len(".lproj")] for path in catalog_root.glob("*.lproj")}
if present != set(locales):
    problems.append(
        f"{catalog_root} locales differ from the canonical set {locales}: "
        f"missing={sorted(set(locales) - present)}, unexpected={sorted(present - set(locales))}"
    )
for locale in locales:
    catalog = catalog_root / f"{locale}.lproj" / english_path.name
    if catalog == english_path:
        continue
    if not catalog.is_file():
        problems.append(f"{catalog} is missing")
        continue
    values = catalog_values(catalog)
    missing = sorted(english.keys() - values.keys())
    if missing:
        problems.append(report(catalog, "is missing keys from the English source catalog", missing))
    unknown = sorted(values.keys() - english.keys())
    if unknown:
        problems.append(report(catalog, "contains keys absent from the English source catalog", unknown))
    mismatched = sorted(
        key for key in english.keys() & values.keys()
        if signature(values[key]) != signature(english[key])
    )
    if mismatched:
        problems.append(report(catalog, "has format placeholders that do not match English", mismatched))
if problems:
    raise SystemExit("\n".join(problems))
PY

    extract_keys "$english" | LC_ALL=C sort >"$work/en.lproj.keys"
}

static_keys() {
    matches --pcre2 --multiline --no-filename --only-matching --replace '$1' \
        '(?:L10n\.tr|String\s*\(\s*localized:|LocalizedStringResource|LocalizedStringKey|Text|Button|Label|Toggle|\.navigationTitle|\.help)\s*\(\s*"([a-z0-9_]+(?:\.[a-z0-9_]+)+)"' \
        "$@" --glob '*.swift'

    python3 - "$@" <<'PY'
import pathlib
import re
import sys

key = re.compile(r'"([a-z0-9_]+(?:\.[a-z0-9_]+)+)"')


def first_argument(source, opening_parenthesis):
    depth = 1
    index = opening_parenthesis + 1
    start = index
    in_string = False
    escaped = False
    while index < len(source):
        character = source[index]
        if in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
        elif character == '"':
            in_string = True
        elif character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
            if depth == 0:
                return source[start:index]
        elif character == "," and depth == 1:
            return source[start:index]
        index += 1
    return ""


for root in map(pathlib.Path, sys.argv[1:]):
    for source_path in [root] if root.is_file() else root.rglob('*.swift'):
        source = source_path.read_text()
        offset = 0
        while (call := source.find("L10n.tr", offset)) != -1:
            opening_parenthesis = call + len("L10n.tr")
            while opening_parenthesis < len(source) and source[opening_parenthesis].isspace():
                opening_parenthesis += 1
            if opening_parenthesis < len(source) and source[opening_parenthesis] == "(":
                for match in key.finditer(first_argument(source, opening_parenthesis)):
                    print(match.group(1))
            offset = opening_parenthesis + 1
PY

    matches --pcre2 --no-filename --only-matching --replace '$1' \
        '(?:nameKey|descriptionKey|titleKey|bodyKey|keyLabelKey|keyHelpKey):\s*"([a-z0-9_]+(?:\.[a-z0-9_]+)+)"' \
        "$@" --glob '*.swift'
}

dynamic_keys() {
    python3 - "$MAC_SOURCE_ROOT" "$PHONE_SOURCE_ROOT" "$1" <<'PY'
import pathlib
import re
import sys

source_root = pathlib.Path(sys.argv[1])
phone_root = pathlib.Path(sys.argv[2])
surface = sys.argv[3]


def without_comments(source):
    result = list(source)
    index = 0
    in_string = False
    escaped = False
    block_depth = 0
    while index < len(source):
        if block_depth:
            if source.startswith("/*", index):
                result[index:index + 2] = "  "
                block_depth += 1
                index += 2
                continue
            if source.startswith("*/", index):
                result[index:index + 2] = "  "
                block_depth -= 1
                index += 2
                continue
            if source[index] != "\n":
                result[index] = " "
            index += 1
            continue
        if in_string:
            if escaped:
                escaped = False
            elif source[index] == "\\":
                escaped = True
            elif source[index] == '"':
                in_string = False
            index += 1
            continue
        if source.startswith("//", index):
            newline = source.find("\n", index)
            end = len(source) if newline == -1 else newline
            result[index:end] = " " * (end - index)
            index = end
            continue
        if source.startswith("/*", index):
            result[index:index + 2] = "  "
            block_depth = 1
            index += 2
            continue
        if source[index] == '"':
            in_string = True
        index += 1
    if block_depth:
        raise SystemExit("unterminated Swift block comment")
    return "".join(result)


def enum_values(relative_path, enum_name):
    source = without_comments((source_root / relative_path).read_text())
    declaration = re.search(rf'enum\s+{re.escape(enum_name)}\b[^{{]*\{{', source)
    if declaration is None:
        raise SystemExit(f"could not find declaration for {enum_name}")

    body_start = declaration.end()
    depth = 1
    index = body_start
    in_string = False
    escaped = False
    while index < len(source) and depth:
        character = source[index]
        if in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
        elif character == '"':
            in_string = True
        elif character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
        index += 1
    if depth:
        raise SystemExit(f"unterminated declaration for {enum_name}")

    body = source[body_start:index - 1]
    values = []
    depth = 0
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if depth == 0 and line.startswith("case "):
            declarations = line.removeprefix("case ").split(",")
            for case_declaration in declarations:
                case = re.fullmatch(
                    r'\s*(\w+)(?:\s*=\s*"([^"]+)")?\s*',
                    case_declaration,
                )
                if case is None:
                    raise SystemExit(
                        f"could not parse {enum_name} case: {case_declaration.strip()}"
                    )
                values.append(case.group(2) or case.group(1))
        depth += line.count("{") - line.count("}")
    if not values:
        raise SystemExit(f"could not find cases for {enum_name}")
    return values


keys = set()
if surface == "iPhone":
    for value in enum_values('DTOs/EntityTypes.swift', 'GoalStatus'):
        keys.add(f"phone.goal.status.{value}")

    home = without_comments((phone_root / 'Views/Home/HomeView.swift').read_text())
    suggestions = re.search(r'\bsuggestions\s*=\s*\[([^]]*)\]', home)
    values = re.findall(r'"([a-z0-9_]+(?:\.[a-z0-9_]+)+)"', suggestions.group(1)) if suggestions else []
    if not values:
        raise SystemExit("could not find home suggestion localization keys")
    keys.update(values)
else:
    for value in enum_values('DTOs/SettingsTypes.swift', 'PhysicalBodyTheme'):
        keys.add(f"settings.appearance.body.theme.{value}.name")
    for value in enum_values(
        'Features/Settings/Editors/GuidedMarkdownEditor.swift',
        'ContextEditorMode',
    ):
        keys.add(f"settings.context_editor.mode.{value}")
    for value in enum_values('Views/Setup/ProactivityStepView.swift', 'ProactivityLevel'):
        for suffix in ('title', 'description'):
            keys.add(f"setup.proactivity.{value}.{suffix}")

    voice_panel = (source_root / 'Features/Settings/Panels/VoicePanel.swift').read_text()
    pause = re.search(
        r'options:\s*\[([^]]+)\],\s*'
        r'label:\s*\{\s*L10n\.tr\("settings\.voice\.pause_sensitivity\.\\\(\$0\)"\)',
        voice_panel,
    )
    if pause is None:
        raise SystemExit("could not find pause-sensitivity localization options")
    for value in re.findall(r'"([a-z0-9_]+)"', pause.group(1)):
        keys.add(f"settings.voice.pause_sensitivity.{value}")

    goals = (source_root / 'Features/Settings/Editors/GoalsEditorDetail.swift').read_text()
    priority = re.search(r'min\(max\(priority,\s*(\d+)\),\s*(\d+)\)', goals)
    if priority is None:
        raise SystemExit("could not find goal-priority localization bounds")
    minimum, maximum = map(int, priority.groups())
    for value in range(minimum, maximum + 1):
        keys.add(f"settings.goals.priority.{value}")

print(*sorted(keys), sep="\n")
PY
}

check_sources() {
    local surface="$1"
    local work="$2"
    shift 2

    static_keys "$@" >"$work/source.keys"
    LC_ALL=C sort -u -o "$work/source.keys" "$work/source.keys"

    local missing
    missing="$(comm -23 "$work/source.keys" "$work/en.lproj.keys")"
    if [[ -n "$missing" ]]; then
        echo "$surface Swift source references localization keys absent from the English catalog:" >&2
        printf '%s\n' "$missing" >&2
        exit 1
    fi

    dynamic_keys "$surface" >"$work/source.dynamic.keys"

    local missing_dynamic
    missing_dynamic="$(comm -23 "$work/source.dynamic.keys" "$work/en.lproj.keys")"
    if [[ -n "$missing_dynamic" ]]; then
        echo "$surface dynamically generated localization keys are absent from the English catalog:" >&2
        printf '%s\n' "$missing_dynamic" >&2
        exit 1
    fi

    LC_ALL=C sort -u \
        "$work/source.keys" \
        "$work/source.dynamic.keys" \
        >"$work/source.used.keys"
    local unused
    unused="$(comm -23 "$work/en.lproj.keys" "$work/source.used.keys")"
    if [[ -n "$unused" ]]; then
        echo "$surface English localization keys have no production Swift reference:" >&2
        printf '%s\n' "$unused" >&2
        exit 1
    fi
}

check_catalogs "$MAC_CATALOG_ROOT/en.lproj/UI.strings" "$TEMP_DIR/mac"
check_catalogs "$PHONE_CATALOG_ROOT/en.lproj/UI.strings" "$TEMP_DIR/phone"
check_catalogs "$PHONE_CATALOG_ROOT/en.lproj/InfoPlist.strings" "$TEMP_DIR/phone-info"

python3 - "$PHONE_SOURCE_ROOT/Resources/Info.plist" "$PHONE_CATALOG_ROOT/en.lproj/InfoPlist.strings" <<'PY'
import pathlib
import plistlib
import re
import sys

info = plistlib.loads(pathlib.Path(sys.argv[1]).read_bytes())
entry = re.compile(r'^\s*"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)";')
english = {}
for line in pathlib.Path(sys.argv[2]).read_text().splitlines():
    match = entry.match(line)
    if match:
        english[match.group(1)] = match.group(2)
usage = {key: value for key, value in info.items() if key.endswith("UsageDescription")}
if english != usage:
    raise SystemExit(f"{sys.argv[2]} must repeat the usage descriptions in {sys.argv[1]} exactly")
PY

if rg -n -i 'TODO[[:space:]]+translate|pending[[:space:]]+translation|back-filled with English' "$MAC_CATALOG_ROOT" "$PHONE_CATALOG_ROOT"; then
    echo "Localization catalogs contain unfinished translation markers." >&2
    exit 1
fi

check_sources Mac "$TEMP_DIR/mac" "$MAC_SOURCE_ROOT"
check_sources iPhone "$TEMP_DIR/phone" "$PHONE_SOURCE_ROOT" "${PHONE_SHARED_SOURCES[@]}"

echo "Localization catalogs and static Swift references are valid."
