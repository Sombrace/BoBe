import importlib.util
import json
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest
from datetime import datetime, timedelta, timezone

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("import_claim", ROOT / "tools/import_claim.py")
assert SPEC is not None and SPEC.loader is not None
CLAIMS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CLAIMS)


def private_key(key):
    return key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                             serialization.NoEncryption()).decode()


class ClaimTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        key = ec.generate_private_key(ec.SECP256R1())
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "SDK importer test")])
        now = datetime.now(timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
                .public_key(key.public_key()).serial_number(x509.random_serial_number())
                .not_valid_before(now - timedelta(minutes=1)).not_valid_after(now + timedelta(days=1))
                .sign(key, hashes.SHA256()).public_bytes(serialization.Encoding.PEM).decode())
        cls.claim = {
            "device_id": "785d18bc-fc41-4e2e-bbe0-54b22ff4d585", "trust_kid": "test",
            "endpoint_uri": "wss://bobe.example:49987/body/v1", "controller_epoch": 1,
            "server_ca_pem": cert, "client_cert_pem": cert, "client_key_pem": private_key(key),
        }

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.project = self.root / "project"
        (self.project / "main").mkdir(parents=True)
        (self.project / "CMakeLists.txt").write_text("project(test)")
        self.path = self.root / "claim.json"
        self.write_claim(self.claim)

    def write_claim(self, value):
        self.path.write_text(json.dumps(value))
        self.path.chmod(0o600)

    def test_private_output_and_ignore_rules(self):
        (self.project / ".gitignore").write_text("/private/\n!private/\n!private/**\n")
        output = CLAIMS.import_claim(self.path, self.project)
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(output.parent.stat().st_mode), 0o700)
        self.assertIn("BOBE_PROVISIONED_DEVICE_ID", output.read_text())
        self.assertTrue((self.project / ".gitignore").read_text().endswith("\n/private/\n"))
        if shutil.which("git"):
            subprocess.run(["git", "init", "-q", str(self.project)], check=True)
            subprocess.run(["git", "-C", str(self.project), "check-ignore", "-q", str(output)], check=True)

    def test_existing_credentials_are_preserved(self):
        output = CLAIMS.import_claim(self.path, self.project)
        original = output.read_bytes()
        with self.assertRaises(ValueError):
            CLAIMS.import_claim(self.path, self.project)
        self.assertEqual(output.read_bytes(), original)

    def test_insecure_claim_and_private_directory_are_rejected(self):
        self.path.chmod(0o644)
        with self.assertRaisesRegex(ValueError, "owner"):
            CLAIMS.import_claim(self.path, self.project)
        self.path.chmod(0o600)
        (self.project / "private").mkdir(mode=0o755)
        with self.assertRaisesRegex(ValueError, "700"):
            CLAIMS.import_claim(self.path, self.project)
        self.assertFalse((self.project / "private/bobe_credentials.h").exists())

    def test_duplicate_fields_and_mismatched_key_are_rejected(self):
        original = self.path.read_text()
        self.path.write_text('{"device_id":"different",' + original[1:])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            CLAIMS.read_claim(self.path)
        self.write_claim({**self.claim, "client_key_pem": private_key(ec.generate_private_key(ec.SECP256R1()))})
        with self.assertRaisesRegex(ValueError, "do not match"):
            CLAIMS.read_claim(self.path)

    def test_invalid_endpoints_and_epochs_are_rejected(self):
        for endpoint in ["ws://bobe.example", "wss://user:secret@bobe.example", "wss://bobe.example:0",
                         "wss://bobe.example:70000", "wss://bo\nbe.example", "wss://bobe.example?secret=yes"]:
            with self.subTest(endpoint=endpoint):
                self.write_claim({**self.claim, "endpoint_uri": endpoint})
                with self.assertRaises(ValueError):
                    CLAIMS.read_claim(self.path)
        for epoch in [True, 0, -1, 9007199254740992]:
            self.write_claim({**self.claim, "controller_epoch": epoch})
            with self.assertRaises(ValueError):
                CLAIMS.read_claim(self.path)

    def test_symlinks_cannot_redirect_secrets_or_ignore_rules(self):
        outside = self.root / "outside"
        outside.mkdir(mode=0o700)
        (self.project / "private").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(ValueError):
            CLAIMS.import_claim(self.path, self.project)
        (self.project / "private").unlink()
        target = outside / "ignore"
        target.write_text("keep")
        (self.project / ".gitignore").symlink_to(target)
        with self.assertRaises(ValueError):
            CLAIMS.import_claim(self.path, self.project)
        self.assertEqual(target.read_text(), "keep")
        self.assertFalse((self.project / "private/bobe_credentials.h").exists())

    @unittest.skipUnless(shutil.which("git"), "requires git")
    def test_tracked_destination_is_rejected_even_if_absent(self):
        subprocess.run(["git", "init", "-q", str(self.project)], check=True)
        private = self.project / "private"
        private.mkdir(mode=0o700)
        output = private / "bobe_credentials.h"
        output.write_text("placeholder")
        subprocess.run(["git", "-C", str(self.project), "add", str(output)], check=True)
        output.unlink()
        with self.assertRaisesRegex(ValueError, "tracked"):
            CLAIMS.import_claim(self.path, self.project)
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
