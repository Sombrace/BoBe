import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from bobe_hil import locks


SNAPSHOT_LOCK = """SNAPSHOT_SOURCE_COMMIT={commit}
SNAPSHOT_IMAGE_SHA256={image}
SNAPSHOT_ELF_SHA256={elf}
SNAPSHOT_APP_ELF_SHA256={elf}
SNAPSHOT_IMAGE_SIZE=1024
SNAPSHOT_BUILD_DIRECTORY=build-bobe-snapshot
"""
LKG_LOCK = """LKG_SOURCE_COMMIT={commit}
LKG_IMAGE_SHA256={image}
LKG_ELF_SHA256={elf}
LKG_APP_ELF_SHA256={elf}
LKG_IMAGE_SIZE=2048
LKG_BUILD_DIRECTORY=build-ondevice-reviewed
"""
COMMIT = "0" * 40
IMAGE = "1" * 64
ELF = "2" * 64


class LockParsingTests(unittest.TestCase):
    def write(self, root: Path, template: str, **fields: str) -> Path:
        path = root / "lock"
        path.write_text(
            template.format(commit=COMMIT, image=IMAGE, elf=ELF, **fields),
            encoding="ascii",
        )
        return path

    def test_snapshot_lock_fields_are_exact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(Path(directory), SNAPSHOT_LOCK)
            values = locks.load_snapshot_lock(path)
        self.assertEqual(values["SNAPSHOT_SOURCE_COMMIT"], COMMIT)
        self.assertEqual(values["SNAPSHOT_IMAGE_SIZE"], "1024")

    def test_lkg_lock_fields_are_exact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(Path(directory), LKG_LOCK)
            values = locks.load_lkg_lock(path)
        self.assertEqual(values["LKG_BUILD_DIRECTORY"], "build-ondevice-reviewed")

    def test_mismatched_elf_digests_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lock"
            path.write_text(
                SNAPSHOT_LOCK.format(
                    commit=COMMIT,
                    image=IMAGE,
                    elf=ELF,
                ).replace(
                    f"SNAPSHOT_APP_ELF_SHA256={ELF}",
                    f"SNAPSHOT_APP_ELF_SHA256={'3' * 64}",
                ),
                encoding="ascii",
            )
            with self.assertRaises(ValueError):
                locks.load_snapshot_lock(path)

    def test_incomplete_lock_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lock"
            path.write_text(
                f"SNAPSHOT_SOURCE_COMMIT={COMMIT}\n",
                encoding="ascii",
            )
            with self.assertRaises(ValueError):
                locks.load_snapshot_lock(path)

    def test_repository_locks_parse(self) -> None:
        root = Path(__file__).resolve().parent.parent / "hil"
        if (root / "bobe-snapshot.lock").is_file():
            locks.load_snapshot_lock(root / "bobe-snapshot.lock")
        if (root / "last-known-good.lock").is_file():
            locks.load_lkg_lock(root / "last-known-good.lock")


class LockCreationTests(unittest.TestCase):
    def test_lock_records_the_snapshot_build_digests(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            build = root / "build-bobe-snapshot"
            build.mkdir()
            (build / "bobe_wb12.bin").write_bytes(b"image")
            (build / "bobe_wb12.elf").write_bytes(b"elf")
            output = root / "bobe-snapshot.lock"
            with (
                mock.patch.object(
                    locks,
                    "inspect_application",
                    return_value={
                        "schema_version": 1,
                        "target": "esp32s3",
                        "project_name": "bobe_wb12",
                        "app_version": "1",
                        "compile_time": "00:00:00",
                        "compile_date": "Jan 01 2030",
                        "idf_version": "v6.0",
                        "app_elf_sha256": ELF,
                        "secure_version": 0,
                        "image_sha256": IMAGE,
                        "image_size": 4096,
                    },
                ),
                mock.patch.object(locks, "sha256_file", return_value=ELF),
            ):
                values = locks.create_snapshot_lock(
                    COMMIT,
                    build,
                    output,
                    "edge",
                )
            self.assertEqual(values["SNAPSHOT_IMAGE_SHA256"], IMAGE)
            self.assertEqual(locks.load_snapshot_lock(output), values)
            manifest = json.loads(
                (build / "bobe_wb12.bin.manifest.json").read_text()
            )
            self.assertEqual(manifest["profile"], "edge")
            self.assertEqual(manifest["source_commit"], COMMIT)
            self.assertFalse(manifest["source_dirty"])

    def test_unexpected_build_directory_name_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            build = root / "build-other"
            build.mkdir()
            with self.assertRaises(ValueError):
                locks.create_snapshot_lock(COMMIT, build, root / "lock", "edge")

    def test_existing_output_is_never_replaced(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            build = root / "build-bobe-snapshot"
            build.mkdir()
            output = root / "lock"
            output.write_text("existing", encoding="ascii")
            with self.assertRaises(ValueError):
                locks.create_snapshot_lock(COMMIT, build, output, "edge")


if __name__ == "__main__":
    unittest.main()
