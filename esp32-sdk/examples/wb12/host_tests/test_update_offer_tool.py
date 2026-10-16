import argparse
import base64
import json
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from tools import create_update_offer


class UpdateOfferToolTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.image = self.root / "app.bin"
        self.image.write_bytes(b"esp-image")
        digest = __import__("hashlib").sha256(self.image.read_bytes()).hexdigest()
        self.artifact = self.root / "app.bin.manifest.json"
        self.metadata = {
            "image_sha256": digest,
            "image_size": self.image.stat().st_size,
            "source_dirty": False,
            "target": "esp32s3",
            "project_name": "bobe_wb12",
            "app_version": "0.3.0",
            "build_number": 3,
        }
        self.artifact.write_text(json.dumps(self.metadata))
        self.key = self.root / "key.pem"
        self.public = self.root / "public.pem"
        subprocess.run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", str(self.key)], check=True, capture_output=True)
        subprocess.run(["openssl", "pkey", "-in", str(self.key), "-pubout", "-out", str(self.public)], check=True, capture_output=True)

    def args(self, **changes: object) -> argparse.Namespace:
        values = dict(
            image=self.image,
            artifact_manifest=self.artifact,
            private_key=self.key,
            output=self.root / "offer.json",
            url="https://updates.example/body.bin",
            key_id="test-key",
            build_number=3,
            current_build_number=2,
            not_before=1_700_000_000,
            expires_at=1_800_000_000,
        )
        values.update(changes)
        return argparse.Namespace(**values)

    def test_builds_canonical_verifiable_offer(self) -> None:
        create_update_offer.build_offer(self.args())
        offer = json.loads((self.root / "offer.json").read_text())
        self.assertEqual(set(offer), {"type", "protocol_major", "manifest", "signature"})
        manifest = json.loads(offer["manifest"])
        self.assertEqual(manifest["build_number"], 3)
        self.assertEqual(offer["manifest"].encode(), create_update_offer.canonical(manifest))
        signature = self.root / "signature.der"
        signature.write_bytes(base64.b64decode(offer["signature"], validate=True))
        source = self.root / "manifest.json"
        source.write_text(offer["manifest"])
        subprocess.run(["openssl", "dgst", "-sha256", "-verify", str(self.public), "-signature", str(signature), str(source)], check=True, capture_output=True)

    def test_rejects_dirty_source(self) -> None:
        self.metadata["source_dirty"] = True
        self.artifact.write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(ValueError, "clean source"):
            create_update_offer.build_offer(self.args())

    def test_rejects_downgrade_and_insecure_url(self) -> None:
        with self.assertRaisesRegex(ValueError, "must increase"):
            create_update_offer.build_offer(self.args(build_number=2))
        with self.assertRaisesRegex(ValueError, "bounded HTTPS"):
            create_update_offer.build_offer(self.args(url="http://updates.example/body.bin"))

    def test_rejects_build_number_the_image_does_not_report(self) -> None:
        with self.assertRaisesRegex(ValueError, "does not match the image"):
            create_update_offer.build_offer(self.args(build_number=4))
        self.assertFalse((self.root / "offer.json").exists())
        del self.metadata["build_number"]
        self.artifact.write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(ValueError, "does not record"):
            create_update_offer.build_offer(self.args())
        self.metadata["build_number"] = True
        self.artifact.write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(ValueError, "does not record"):
            create_update_offer.build_offer(self.args(build_number=1, current_build_number=0))

    def test_rejects_values_the_firmware_cannot_parse(self) -> None:
        with self.assertRaisesRegex(ValueError, "build number is outside"):
            create_update_offer.build_offer(
                self.args(build_number=create_update_offer.MAX_BUILD_NUMBER + 1)
            )
        with self.assertRaisesRegex(ValueError, "validity window"):
            create_update_offer.build_offer(
                self.args(expires_at=create_update_offer.MAX_SAFE_INTEGER + 1)
            )
        with self.assertRaisesRegex(ValueError, "key ID is invalid"):
            create_update_offer.build_offer(self.args(key_id="é" * 17))
        self.metadata["app_version"] = "v 1"
        self.artifact.write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(ValueError, "artifact version"):
            create_update_offer.build_offer(self.args())

    def test_rejects_generated_documents_over_bodylink_limits(self) -> None:
        with (
            mock.patch.object(create_update_offer, "MAX_MANIFEST_BYTES", 1),
            self.assertRaisesRegex(ValueError, "manifest exceeds"),
        ):
            create_update_offer.build_offer(self.args())
        with (
            mock.patch.object(create_update_offer, "MAX_OFFER_BYTES", 1),
            self.assertRaisesRegex(ValueError, "offer exceeds"),
        ):
            create_update_offer.build_offer(self.args())


if __name__ == "__main__":
    unittest.main()
