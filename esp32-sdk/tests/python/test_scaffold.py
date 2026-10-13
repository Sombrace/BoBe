import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SDK_ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("new_project", SDK_ROOT / "tools/new_project.py")
assert SPEC is not None and SPEC.loader is not None
SCAFFOLD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCAFFOLD)


class ScaffoldTests(unittest.TestCase):
    def test_local_consumer_has_no_private_data_or_copied_sdk(self):
        with tempfile.TemporaryDirectory() as directory:
            output = SCAFFOLD.create_project("my_keyboard", Path(directory) / "project")
            manifest = json.loads((output / "main/idf_component.yml").read_text())
            self.assertEqual(manifest["dependencies"]["bobe_client"]["path"], str(SDK_ROOT / "components/bobe_client"))
            self.assertFalse((output / "private").exists())
            self.assertFalse((output / "components/bobe_client").exists())
            self.assertIn("project(my_keyboard)", (output / "CMakeLists.txt").read_text())

    def test_existing_files_are_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "existing"
            output.mkdir()
            (output / "mine").write_text("keep")
            with self.assertRaises(ValueError):
                SCAFFOLD.create_project("my_keyboard", output)
            self.assertEqual((output / "mine").read_text(), "keep")

    def test_git_consumption_is_explicit_and_pinned(self):
        with tempfile.TemporaryDirectory() as directory:
            output = SCAFFOLD.create_project("my_protocol", Path(directory) / "project", "protocol", "esp32c6",
                git_url="https://example.test/BoBe.git", git_ref="abc123")
            manifest = json.loads((output / "main/idf_component.yml").read_text())
            self.assertEqual(manifest["dependencies"]["bobe_protocol"]["version"], "abc123")
            self.assertEqual(manifest["dependencies"]["bobe_protocol"]["git"], "https://example.test/BoBe.git")
            self.assertEqual(manifest["dependencies"]["bobe_protocol"]["path"], "esp32-sdk/components/bobe_protocol")
            self.assertIn('set(IDF_TARGET "esp32c6"', (output / "CMakeLists.txt").read_text())

    def test_target_and_name_are_validated_before_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "project"
            for name, template, target in [("bad;command", "protocol", "esp32"), ("valid", "wb12", "esp32c3")]:
                with self.assertRaises(ValueError):
                    SCAFFOLD.create_project(name, output, template, target)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
