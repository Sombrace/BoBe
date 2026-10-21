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


def elf(machine: int) -> bytes:
    return b"\x7fELF\x02\x01\x01" + bytes(9) + (2).to_bytes(2, "little") + machine.to_bytes(2, "little") + bytes(44)


def mach_o(cpu: int) -> bytes:
    return b"\xcf\xfa\xed\xfe" + cpu.to_bytes(4, "little") + bytes(24)


def universal(*cpus: int) -> bytes:
    header = b"\xca\xfe\xba\xbe" + len(cpus).to_bytes(4, "big")
    for index, cpu in enumerate(cpus):
        header += cpu.to_bytes(4, "big") + bytes(4) + (4096 * (index + 1)).to_bytes(4, "big") + bytes(8)
    return header


ELF_X86_64 = elf(62)
ELF_AARCH64 = elf(183)
MACH_O_ARM64 = mach_o(0x0100000C)
MACH_O_X86_64 = mach_o(0x01000007)


def create_archive(
    path: pathlib.Path,
    omitted: str | None = None,
    root: str = "bobe-daemon-9.8.7-linux-x86_64",
    executables: dict[str, bytes] | None = None,
    checksum: str | None = "valid",
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
    contents = {"bin/bobe": ELF_X86_64, "bin/node": ELF_X86_64}
    contents.update(executables or {})
    module = load_module()
    runtime_files = module.REQUIRED_RUNTIME_FILES
    if str(compatibility.get("platform", "")).startswith("linux-"):
        runtime_files = runtime_files | module.LINUX_RUNTIME_FILES
    with tarfile.open(path, "w:gz") as bundle:
        add_bytes(bundle, f"{root}/compatibility.json", json.dumps(compatibility).encode())
        for name in runtime_files:
            if name != omitted:
                add_bytes(bundle, f"{root}/{name}", contents.get(name, b"runtime-fixture"))
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    lines = {
        "valid": f"{digest}  {path.name}\n",
        "wrong-digest": f"{hashlib.sha256(b'other').hexdigest()}  {path.name}\n",
        "wrong-name": f"{digest}  other.tar.gz\n",
    }
    sidecar = path.with_name(f"{path.name}.sha256")
    sidecar.unlink(missing_ok=True)
    if checksum is not None:
        sidecar.write_text(lines[checksum])
    return compatibility


def expect_rejection(module, archive: pathlib.Path, reason: str) -> None:
    try:
        module.create_manifest(archive)
    except SystemExit as error:
        assert reason in str(error), error
    else:
        raise AssertionError(f"{archive.name} was accepted; expected: {reason}")


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

        mac = directory / "bobe-daemon-9.8.7-darwin-arm64.tar.gz"
        create_archive(
            mac,
            root="bobe-daemon-9.8.7-darwin-arm64",
            executables={"bin/bobe": MACH_O_ARM64, "bin/node": universal(0x01000007, 0x0100000C)},
            platform="darwin-arm64",
        )
        assert module.create_manifest(mac)["platform"] == "darwin-arm64"

        for executables, platform, reason in (
            ({"bin/bobe": ELF_AARCH64}, "linux-x86_64", "bin/bobe is not built for linux-x86_64"),
            ({"bin/node": MACH_O_ARM64}, "linux-x86_64", "bin/node is not built for linux-x86_64"),
            ({"bin/bobe": b"runtime-fixture"}, "linux-x86_64", "bin/bobe is not built for linux-x86_64"),
            (
                {"bin/bobe": MACH_O_X86_64, "bin/node": MACH_O_ARM64},
                "darwin-arm64",
                "bin/bobe is not built for darwin-arm64",
            ),
            (
                {"bin/bobe": MACH_O_ARM64, "bin/node": universal(0x01000007)},
                "darwin-arm64",
                "bin/node is not built for darwin-arm64",
            ),
        ):
            mismatched = directory / f"bobe-daemon-9.8.7-{platform}.tar.gz"
            create_archive(mismatched, root=f"bobe-daemon-9.8.7-{platform}", executables=executables, platform=platform)
            expect_rejection(module, mismatched, reason)

        renamed = directory / "bobe-daemon-9.8.6-linux-x86_64.tar.gz"
        create_archive(renamed)
        expect_rejection(module, renamed, "archive name bobe-daemon-9.8.6-linux-x86_64.tar.gz does not match")
        rooted = directory / "bobe-daemon-9.8.7-linux-x86_64.tar.gz"
        create_archive(rooted, root="bobe-daemon-9.8.6-linux-x86_64")
        expect_rejection(module, rooted, "archive root bobe-daemon-9.8.6-linux-x86_64 does not match")
        create_archive(rooted, version="9.8.6")
        expect_rejection(module, rooted, "does not match compatibility.json (bobe-daemon-9.8.6-linux-x86_64)")
        create_archive(rooted, platform="linux-aarch64", executables={"bin/bobe": ELF_AARCH64, "bin/node": ELF_AARCH64})
        expect_rejection(module, rooted, "does not match compatibility.json (bobe-daemon-9.8.7-linux-aarch64)")

        for checksum, reason in (
            (None, "checksum not found"),
            ("wrong-digest", "does not match the SHA-256"),
            ("wrong-name", "describes other.tar.gz"),
        ):
            create_archive(rooted, checksum=checksum)
            expect_rejection(module, rooted, reason)

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
            "bin/agent/lib/libatomic.so.1",
            "bin/agent/licenses/libatomic-LICENSE",
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
