import ast
import re
import struct
import sys
from pathlib import Path

PAIRS = {"{": "}", "[": "]", "(": ")"}
SWIFT_TYPE = (
    r"^[ \t]*(?:@\w+(?:\([^)\n]*\))?[ \t]+)*"
    r"(?:(?:public|internal|private|fileprivate|final|nonisolated|indirect)[ \t]+)*"
    r"(enum|struct|class|actor|extension)[ \t]+{name}\b[^{{\n]*\{{"
)


def snake(name):
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name).lower()


class Contract:
    def __init__(self, root, title):
        self.root = Path(root)
        self.title = title
        self.failures = []
        self.sources = {}
        self.swift_types = None

    def fail(self, message):
        raise SystemExit(f"{self.title} contract parser: {message}")

    def read(self, path):
        path = self.root / path
        if path not in self.sources:
            if not path.is_file():
                self.fail(f"source missing: {path}")
            self.sources[path] = path.read_text(encoding="utf-8")
        return self.sources[path]

    def single(self, label, values):
        if len(values) != 1:
            self.fail(f"expected one {label}, found {len(values)}")
        return values[0]

    def find(self, label, pattern, text, flags=re.MULTILINE):
        return self.single(label, re.findall(pattern, text, flags))

    def findall(self, label, pattern, text, flags=re.MULTILINE):
        values = re.findall(pattern, text, flags)
        if not values:
            self.fail(f"found no {label}")
        return values

    def require(self, label, snippet, text):
        if snippet not in text:
            self.drift(label)

    def block_at(self, label, text, start):
        opener = text[start - 1]
        closer = PAIRS[opener]
        depth = 1
        index = start
        while index < len(text):
            if text.startswith("//", index):
                newline = text.find("\n", index)
                index = len(text) if newline == -1 else newline
                continue
            character = text[index]
            if character == '"':
                index = self.string_end(label, text, index)
                continue
            if character == opener:
                depth += 1
            elif character == closer:
                depth -= 1
                if depth == 0:
                    return text[start:index]
            index += 1
        self.fail(f"unterminated {label}")

    def string_end(self, label, text, index):
        if text.startswith('"""', index):
            end = text.find('"""', index + 3)
            if end == -1:
                self.fail(f"unterminated string in {label}")
            return end + 3
        index += 1
        while index < len(text):
            if text[index] == "\\":
                index += 2
                continue
            if text[index] == '"':
                return index + 1
            index += 1
        self.fail(f"unterminated string in {label}")

    def block(self, label, text, opening):
        starts = [match.end() for match in re.finditer(opening, text, re.MULTILINE)]
        return self.block_at(label, text, self.single(label, starts))

    def integer(self, label, expression):
        text = re.sub(r"^U?INT(?:8|16|32|64)_C\((.+)\)$", r"\1", expression.strip())
        text = text.replace("_", "")
        product = 1
        for factor in text.split("*"):
            factor = re.sub(r"(?:[ui](?:8|16|32|64|size)|[uUlL]+)$", "", factor.strip())
            try:
                product *= int(factor, 0)
            except ValueError:
                self.fail(f"{label} is not an integer literal: {expression.strip()}")
        return product

    def string(self, label, expression):
        match = re.fullmatch(r'"((?:[^"\\]|\\.)*)"', expression.strip())
        if match is None:
            self.fail(f"{label} is not a string literal: {expression.strip()}")
        return match.group(1)

    def rust_const(self, path, name, module=None):
        text = self.read(path)
        if module is not None:
            text = self.block(
                f"Rust module {module} in {path}",
                text,
                rf"^[ \t]*(?:pub(?:\([a-z]+\))?[ \t]+)?mod[ \t]+{module}[ \t]*\{{",
            )
        return self.find(
            f"Rust const {name} in {path}",
            rf"^[ \t]*(?:pub(?:\([a-z]+\))?[ \t]+)?const[ \t]+{name}[ \t]*:[^=\n]+=\s*([^;]+?)\s*;",
            text,
        )

    def rust_enum(self, path, name):
        body = self.block(
            f"Rust enum {name} in {path}",
            self.read(path),
            rf"^(?:pub(?:\([a-z]+\))?[ \t]+)?enum[ \t]+{name}[ \t]*\{{",
        )
        return {snake(variant) for variant in self.findall(f"variants of Rust enum {name}", r"^    ([A-Z][A-Za-z0-9]*)\b", body)}

    def rust_production(self, path):
        return self.read(path).split("#[cfg(test)]", 1)[0]

    def swift_blocks(self, owner, kinds=("enum", "struct", "class", "actor", "extension")):
        if self.swift_types is None:
            self.swift_types = sorted((self.root / "BoBeMacUI/BoBe").rglob("*.swift"))
            if not self.swift_types:
                self.fail("found no Swift sources under BoBeMacUI/BoBe")
        blocks = []
        pattern = re.compile(SWIFT_TYPE.format(name=re.escape(owner)), re.MULTILINE)
        for path in self.swift_types:
            text = self.read(path)
            if owner not in text:
                continue
            for match in pattern.finditer(text):
                if match.group(1) in kinds:
                    blocks.append(self.block_at(f"Swift {owner} in {path}", text, match.end()))
        return blocks

    def swift_type(self, owner):
        return self.single(f"Swift type {owner}", self.swift_blocks(owner, ("enum", "struct", "class", "actor")))

    def swift_static(self, owner, name):
        values = []
        for body in self.swift_blocks(owner):
            values.extend(
                re.findall(
                    rf"^[ \t]*(?:(?:public|internal|private|fileprivate|nonisolated)[ \t]+)*static[ \t]+let[ \t]+{name}[ \t]*(?::[^=\n]+)?=[ \t]*(.+?)[ \t]*$",
                    body,
                    re.MULTILINE,
                )
            )
        return self.single(f"Swift {owner}.{name}", values)

    def swift_enum(self, owner):
        body = self.single(f"Swift enum {owner}", self.swift_blocks(owner, ("enum",)))
        values = []
        depth = 0
        for raw in body.splitlines():
            line = raw.strip()
            if depth == 0 and line.startswith("case "):
                for declaration in line[len("case "):].split(","):
                    case = re.fullmatch(r'\s*(\w+)(?:\s*=\s*"([^"]+)")?\s*', declaration)
                    if case is None:
                        self.fail(f"cannot parse Swift {owner} case: {declaration.strip()}")
                    values.append(case.group(2) or case.group(1))
            depth += line.count("{") - line.count("}")
        if not values:
            self.fail(f"found no cases of Swift enum {owner}")
        return set(values)

    def c_define(self, path, name):
        return self.find(
            f"C define {name} in {path}",
            rf"^[ \t]*#[ \t]*define[ \t]+{name}[ \t]+(.+?)[ \t]*$",
            self.read(path),
        )

    def c_enum_value(self, path, name):
        return self.find(
            f"C enumerator {name} in {path}",
            rf"^[ \t]*{name}[ \t]*=[ \t]*([^,\n]+?)[ \t]*,?[ \t]*$",
            self.read(path),
        )

    def python_constants(self, path):
        values = {}
        counts = {}
        for node in ast.parse(self.read(path)).body:
            if not (isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)):
                continue
            name = node.targets[0].id
            counts[name] = counts.get(name, 0) + 1
            try:
                values[name] = self.python_value(node.value, values)
            except ValueError:
                pass
        for name, count in counts.items():
            if count > 1:
                values.pop(name, None)
        return values

    def python_value(self, node, values):
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, str)):
            return node.value
        if isinstance(node, ast.Name) and node.id in values:
            return values[node.id]
        if isinstance(node, ast.BinOp) and isinstance(node.op, (ast.Mult, ast.Add, ast.Sub)):
            left = self.python_value(node.left, values)
            right = self.python_value(node.right, values)
            if isinstance(left, int) and isinstance(right, int):
                if isinstance(node.op, ast.Mult):
                    return left * right
                if isinstance(node.op, ast.Add):
                    return left + right
                return left - right
        if (
            isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "struct"
            and node.func.attr == "Struct"
            and len(node.args) == 1
            and isinstance(node.args[0], ast.Constant)
            and isinstance(node.args[0].value, str)
        ):
            return struct.Struct(node.args[0].value)
        raise ValueError("unsupported constant expression")

    def python_constant(self, constants, path, name):
        if name not in constants:
            self.fail(f"expected one literal Python constant {name} in {path}")
        return constants[name]

    def drift(self, message):
        self.failures.append(f"{self.title} drift: {message}")

    def expect(self, label, *values):
        if len(values) < 2 or any(value is None or value == "" for value in values):
            self.drift(f"{label} — a value is missing: {values!r}")
        elif any(value != values[0] for value in values[1:]):
            self.drift(f"{label} — " + " != ".join(repr(value) for value in values))

    def expect_set(self, label, expected, actual, expected_name="Rust", actual_name="Swift"):
        if not expected or not actual:
            self.drift(f"{label} — an extracted set is empty ({expected_name}={sorted(expected)}, {actual_name}={sorted(actual)})")
        elif expected != actual:
            self.drift(
                f"{label} — missing from {actual_name}={sorted(expected - actual)}, "
                f"extra in {actual_name}={sorted(actual - expected)}"
            )

    def expect_subset(self, label, subset, superset, subset_name, superset_name):
        if not subset or not superset:
            self.drift(f"{label} — an extracted set is empty ({subset_name}={sorted(subset)}, {superset_name}={sorted(superset)})")
        elif not subset <= superset:
            self.drift(f"{label} — {subset_name} uses {sorted(subset - superset)} unknown to {superset_name}")

    def finish(self, success):
        if self.failures:
            print("\n".join(self.failures), file=sys.stderr)
            raise SystemExit(1)
        print(success)
