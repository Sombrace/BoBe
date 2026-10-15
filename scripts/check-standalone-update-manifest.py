#!/usr/bin/env python3

import hashlib
import importlib.util
import io
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tarfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/create-standalone-update-manifest.py"
sys.dont_write_bytecode = True


def load_module():
    spec = importlib.util.spec_from_file_location("standalone_manifest", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load standalone manifest script")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def add_bytes(bundle: tarfile.TarFile, name: str, data: bytes) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = 0o644
    bundle.addfile(info, io.BytesIO(data))


def create_archive(
    path: pathlib.Path,
    omitted: str | None = None,
    **overrides: object,
) -> dict[str, object]:
    compatibility = {
        "schema": 1,
        "version": "9.8.7",
        "platform": "linux-x86_64",
        "database_schema": 2,
        "database_schema_min": 1,
        "attach_protocol_min": 1,
        "attach_protocol_max": 1,
    }
    compatibility.update(overrides)
    compatibility = {key: value for key, value in compatibility.items() if value is not None}
    with tarfile.open(path, "w:gz") as bundle:
        add_bytes(
            bundle,
            "bobe-daemon-9.8.7-linux-x86_64/compatibility.json",
            json.dumps(compatibility).encode(),
        )
        for name in load_module().REQUIRED_RUNTIME_FILES:
            if name != omitted:
                add_bytes(bundle, f"bobe-daemon-9.8.7-linux-x86_64/{name}", b"runtime-fixture")
    return compatibility


def main() -> None:
    module = load_module()
    directory = ROOT / "build" / f"manifest-check-{os.getpid()}"
    directory.mkdir(mode=0o700, parents=True)
    try:
        archive = directory / "bobe-daemon-9.8.7-linux-x86_64.tar.gz"
        compatibility = create_archive(archive)
        output = directory / "manifest.json"
        subprocess.run(
            [sys.executable, str(SCRIPT), str(archive), "--output", str(output)],
            check=True,
            stdout=subprocess.PIPE,
            text=True,
        )
        encoded = output.read_bytes()
        manifest = json.loads(encoded)
        expected = {
            "schema": 1,
            "version": compatibility["version"],
            "platform": compatibility["platform"],
            "database_schema": compatibility["database_schema"],
            "database_schema_min": compatibility["database_schema_min"],
            "attach_protocol_min": compatibility["attach_protocol_min"],
            "attach_protocol_max": compatibility["attach_protocol_max"],
            "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
            "archive": archive.name,
        }
        assert manifest == expected
        assert encoded == json.dumps(expected, separators=(",", ":")).encode()

        for overrides, reason in (
            ({"database_schema_min": None}, "missing: database_schema_min"),
            ({"database_schema_min": "1"}, "database_schema_min must be an integer"),
            ({"database_schema_min": 3}, "database_schema_min must not exceed database_schema"),
        ):
            incompatible = directory / "incompatible.tar.gz"
            create_archive(incompatible, **overrides)
            try:
                module.inspect_archive(incompatible)
            except SystemExit as error:
                assert reason in str(error), error
            else:
                raise AssertionError(f"archive with {overrides} was accepted")

        unsafe = directory / "unsafe.tar.gz"
        with tarfile.open(unsafe, "w:gz") as bundle:
            add_bytes(bundle, "../compatibility.json", b"{}")
        try:
            module.inspect_archive(unsafe)
        except SystemExit as error:
            assert "unsafe archive path" in str(error)
        else:
            raise AssertionError("unsafe archive path was accepted")

        symlink = directory / "symlink.tar.gz"
        with tarfile.open(symlink, "w:gz") as bundle:
            info = tarfile.TarInfo("root/bin/bobe")
            info.type = tarfile.SYMTYPE
            info.linkname = "/outside"
            bundle.addfile(info)
        try:
            module.inspect_archive(symlink)
        except SystemExit as error:
            assert "unsupported archive entry" in str(error)
        else:
            raise AssertionError("symlink archive entry was accepted")

        for required in (
            "bin/node",
            "bin/agent/dist/main.js",
            "bin/agent/assets/chat.md",
            "bin/agent/bin/rg",
            "bin/agent/bin/fd",
            "bin/agent/node_modules/quickjs-wasi/quickjs.wasm",
            "bin/agent/node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm",
        ):
            incomplete = directory / "incomplete.tar.gz"
            create_archive(incomplete, omitted=required)
            try:
                module.inspect_archive(incomplete)
            except SystemExit as error:
                assert required in str(error)
            else:
                raise AssertionError(f"archive without {required} was accepted")
    finally:
        shutil.rmtree(directory)
    print("Standalone update manifest and pi asset contract passed")


if __name__ == "__main__":
    main()
