use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde_json::Value;

pub(crate) const MAX_OFFER_BYTES: usize = 4_096;
const MAX_MANIFEST_BYTES: usize = 2_048;
const MAX_SIGNATURE_BASE64: usize = 128;
const DEVICE_MODEL: &str = "wonderboy-wb12";
const SCHEMA_VERSION: u64 = 1;
const MAX_IMAGE_BYTES: u64 = 5 * 1_024 * 1_024;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_VERSION_LEN: usize = 31;
const MAX_URL_LEN: usize = 256;
const MAX_KEY_ID_LEN: usize = 32;
const SHA256_HEX_LEN: usize = 64;

const ENVELOPE_FIELDS: [&str; 4] = ["type", "protocol_major", "manifest", "signature"];
const MANIFEST_FIELDS: [&str; 10] = [
    "schema",
    "device_model",
    "version",
    "build_number",
    "image_size",
    "image_sha256",
    "url",
    "key_id",
    "not_before",
    "expires_at",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OfferError {
    Read(String),
    TooLarge(usize),
    Malformed(&'static str),
    UnknownField(String),
    DuplicateField(String),
    MissingField(&'static str),
    WrongType(&'static str),
    Signature(&'static str),
    Manifest(&'static str),
}

impl fmt::Display for OfferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(detail) => write!(formatter, "cannot read update offer: {detail}"),
            Self::TooLarge(len) => write!(
                formatter,
                "update offer is {len} bytes, over the {MAX_OFFER_BYTES}-byte body control limit"
            ),
            Self::Malformed(detail) => write!(formatter, "malformed update offer: {detail}"),
            Self::UnknownField(name) => write!(formatter, "unknown field `{name}`"),
            Self::DuplicateField(name) => write!(formatter, "duplicate field `{name}`"),
            Self::MissingField(name) => write!(formatter, "missing field `{name}`"),
            Self::WrongType(name) => write!(formatter, "field `{name}` has the wrong JSON type"),
            Self::Signature(detail) => write!(formatter, "invalid signature: {detail}"),
            Self::Manifest(detail) => write!(formatter, "invalid manifest: {detail}"),
        }
    }
}

impl std::error::Error for OfferError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfferManifest {
    pub(crate) device_model: String,
    pub(crate) version: String,
    pub(crate) build_number: u32,
    pub(crate) image_size: u32,
    pub(crate) image_sha256: String,
    pub(crate) url: String,
    pub(crate) key_id: String,
    pub(crate) not_before: i64,
    pub(crate) expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateOffer {
    envelope: String,
    manifest: OfferManifest,
}

impl UpdateOffer {
    pub(crate) fn envelope(&self) -> &str {
        &self.envelope
    }

    pub(crate) const fn manifest(&self) -> &OfferManifest {
        &self.manifest
    }

    pub(crate) const fn build_number(&self) -> u32 {
        self.manifest.build_number
    }

    pub(crate) fn load(path: &Path) -> Result<Self, OfferError> {
        let metadata =
            std::fs::symlink_metadata(path).map_err(|error| OfferError::Read(error.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(OfferError::Read("offer must be a regular file".to_owned()));
        }
        if metadata.len() > MAX_OFFER_BYTES as u64 {
            return Err(OfferError::TooLarge(metadata.len() as usize));
        }

        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| OfferError::Read(error.to_string()))?;
        if !file
            .metadata()
            .map_err(|error| OfferError::Read(error.to_string()))?
            .is_file()
        {
            return Err(OfferError::Read("offer must be a regular file".to_owned()));
        }
        let mut raw = String::new();
        file.take(MAX_OFFER_BYTES as u64 + 1)
            .read_to_string(&mut raw)
            .map_err(|error| OfferError::Read(error.to_string()))?;
        Self::parse(&raw)
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, OfferError> {
        let envelope = raw.trim().to_owned();
        if envelope.len() > MAX_OFFER_BYTES {
            return Err(OfferError::TooLarge(envelope.len()));
        }
        if envelope.is_empty() {
            return Err(OfferError::Malformed("empty document"));
        }

        let keys = object_keys(&envelope)?;
        reject_unknown_or_duplicate(&keys, &ENVELOPE_FIELDS)?;

        let value: Value = serde_json::from_str(&envelope)
            .map_err(|_ignored| OfferError::Malformed("not valid JSON"))?;
        let object = value
            .as_object()
            .ok_or(OfferError::Malformed("not a JSON object"))?;

        match object.get("type").and_then(Value::as_str) {
            Some("update.offer") => {}
            Some(_) => return Err(OfferError::Malformed("type must be \"update.offer\"")),
            None => return Err(OfferError::MissingField("type")),
        }
        match object.get("protocol_major") {
            Some(protocol) if protocol.as_u64() == Some(1) => {}
            Some(_) => {
                return Err(OfferError::Malformed(
                    "protocol_major must be the integer 1",
                ));
            }
            None => return Err(OfferError::MissingField("protocol_major")),
        }

        let manifest_text = object
            .get("manifest")
            .ok_or(OfferError::MissingField("manifest"))?
            .as_str()
            .ok_or(OfferError::WrongType("manifest"))?;
        let signature = object
            .get("signature")
            .ok_or(OfferError::MissingField("signature"))?
            .as_str()
            .ok_or(OfferError::WrongType("signature"))?;

        if manifest_text.is_empty() || manifest_text.len() > MAX_MANIFEST_BYTES {
            return Err(OfferError::Manifest("manifest string length out of range"));
        }
        validate_signature(signature)?;
        let manifest = parse_manifest(manifest_text)?;

        Ok(Self { envelope, manifest })
    }
}

fn validate_signature(signature: &str) -> Result<(), OfferError> {
    if signature.is_empty() || signature.len() > MAX_SIGNATURE_BASE64 {
        return Err(OfferError::Signature("base64 length out of range"));
    }
    let decoded = BASE64
        .decode(signature)
        .map_err(|_ignored| OfferError::Signature("not standard base64"))?;
    if decoded.is_empty() || decoded.len() > 96 {
        return Err(OfferError::Signature("decoded length out of range"));
    }
    validate_ecdsa_der(&decoded)
}

fn validate_ecdsa_der(der: &[u8]) -> Result<(), OfferError> {
    let mut cursor = der.iter().copied();
    if cursor.next() != Some(0x30) {
        return Err(OfferError::Signature("not a DER SEQUENCE"));
    }
    let length = cursor
        .next()
        .ok_or(OfferError::Signature("truncated DER length"))?;
    if usize::from(length) != der.len() - 2 || length & 0x80 != 0 {
        return Err(OfferError::Signature("DER length mismatch"));
    }
    let mut rest = &der[2..];
    for _ in 0..2 {
        let (&tag, tail) = rest
            .split_first()
            .ok_or(OfferError::Signature("truncated DER INTEGER"))?;
        if tag != 0x02 {
            return Err(OfferError::Signature("expected DER INTEGER"));
        }
        let (&int_len, tail) = tail
            .split_first()
            .ok_or(OfferError::Signature("truncated DER INTEGER length"))?;
        let int_len = usize::from(int_len);
        if int_len == 0 || int_len > 33 || tail.len() < int_len {
            return Err(OfferError::Signature("bad DER INTEGER length"));
        }
        rest = &tail[int_len..];
    }
    if rest.is_empty() {
        Ok(())
    } else {
        Err(OfferError::Signature("trailing DER bytes"))
    }
}

fn parse_manifest(text: &str) -> Result<OfferManifest, OfferError> {
    let keys = object_keys(text)?;
    reject_unknown_or_duplicate(&keys, &MANIFEST_FIELDS)?;
    let value: Value =
        serde_json::from_str(text).map_err(|_ignored| OfferError::Manifest("not valid JSON"))?;
    let object = value
        .as_object()
        .ok_or(OfferError::Manifest("not a JSON object"))?;
    for field in MANIFEST_FIELDS {
        if !object.contains_key(field) {
            return Err(OfferError::MissingField(field));
        }
    }

    let unsigned = |name: &'static str| -> Result<u64, OfferError> {
        let field = object.get(name).ok_or(OfferError::MissingField(name))?;
        if !field.is_u64() {
            return Err(OfferError::WrongType(name));
        }
        field.as_u64().ok_or(OfferError::WrongType(name))
    };
    let signed = |name: &'static str| -> Result<i64, OfferError> {
        let field = object.get(name).ok_or(OfferError::MissingField(name))?;
        if !field.is_i64() {
            return Err(OfferError::WrongType(name));
        }
        field.as_i64().ok_or(OfferError::WrongType(name))
    };
    let text_field = |name: &'static str| -> Result<String, OfferError> {
        object
            .get(name)
            .ok_or(OfferError::MissingField(name))?
            .as_str()
            .map(str::to_owned)
            .ok_or(OfferError::WrongType(name))
    };

    if unsigned("schema")? != SCHEMA_VERSION {
        return Err(OfferError::Manifest("schema must be 1"));
    }
    let device_model = text_field("device_model")?;
    if device_model != DEVICE_MODEL {
        return Err(OfferError::Manifest("device_model is not wonderboy-wb12"));
    }
    let version = text_field("version")?;
    if version.is_empty()
        || version.len() > MAX_VERSION_LEN
        || !version.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(OfferError::Manifest(
            "version must be bounded printable ASCII",
        ));
    }
    let build_number = u32::try_from(unsigned("build_number")?)
        .map_err(|_ignored| OfferError::Manifest("build_number out of range"))?;
    if build_number == 0 {
        return Err(OfferError::Manifest("build_number must be positive"));
    }
    let image_size_raw = unsigned("image_size")?;
    if image_size_raw == 0 || image_size_raw > MAX_IMAGE_BYTES {
        return Err(OfferError::Manifest("image_size out of range"));
    }
    let image_size = u32::try_from(image_size_raw)
        .map_err(|_ignored| OfferError::Manifest("image_size out of range"))?;
    let image_sha256 = text_field("image_sha256")?;
    if image_sha256.len() != SHA256_HEX_LEN
        || !image_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(OfferError::Manifest(
            "image_sha256 must be 64 lowercase hex digits",
        ));
    }
    let url = text_field("url")?;
    if !url.starts_with("https://") || url.len() > MAX_URL_LEN {
        return Err(OfferError::Manifest("url must be https and bounded"));
    }
    let key_id = text_field("key_id")?;
    if key_id.is_empty() || key_id.len() > MAX_KEY_ID_LEN {
        return Err(OfferError::Manifest("key_id length out of range"));
    }
    let not_before = signed("not_before")?;
    let expires_at = signed("expires_at")?;
    if not_before < 0 || expires_at <= not_before || expires_at > MAX_SAFE_INTEGER {
        return Err(OfferError::Manifest(
            "validity window must be a positive, safe-integer half-open interval",
        ));
    }

    Ok(OfferManifest {
        device_model,
        version,
        build_number,
        image_size,
        image_sha256,
        url,
        key_id,
        not_before,
        expires_at,
    })
}

fn object_keys(text: &str) -> Result<Vec<String>, OfferError> {
    struct Keys(Vec<String>);

    impl<'de> Deserialize<'de> for Keys {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct KeyVisitor;

            impl<'de> serde::de::Visitor<'de> for KeyVisitor {
                type Value = Keys;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a JSON object")
                }

                fn visit_map<M: serde::de::MapAccess<'de>>(
                    self,
                    mut map: M,
                ) -> Result<Keys, M::Error> {
                    let mut keys = Vec::new();
                    while let Some(key) = map.next_key::<String>()? {
                        keys.push(key);
                        let _ignored = map.next_value::<serde::de::IgnoredAny>()?;
                    }
                    Ok(Keys(keys))
                }
            }

            deserializer.deserialize_map(KeyVisitor)
        }
    }

    serde_json::from_str::<Keys>(text)
        .map(|keys| keys.0)
        .map_err(|_ignored| OfferError::Malformed("not a JSON object"))
}

fn reject_unknown_or_duplicate(keys: &[String], allowed: &[&str]) -> Result<(), OfferError> {
    for (index, key) in keys.iter().enumerate() {
        if !allowed.contains(&key.as_str()) {
            return Err(OfferError::UnknownField(key.clone()));
        }
        if keys[..index].contains(key) {
            return Err(OfferError::DuplicateField(key.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    fn signature_der() -> String {
        let mut der = vec![0x30, 0x44, 0x02, 0x20];
        der.extend(std::iter::repeat_n(0x11, 32));
        der.extend([0x02, 0x20]);
        der.extend(std::iter::repeat_n(0x22, 32));
        BASE64.encode(der)
    }

    fn manifest_json() -> String {
        format!(
            "{{\"schema\":1,\"device_model\":\"wonderboy-wb12\",\"version\":\"0.3.0\",\
\"build_number\":3,\"image_size\":2000000,\"image_sha256\":\"{}\",\
\"url\":\"https://updates.example/bobe.bin\",\"key_id\":\"prod-2026-01\",\
\"not_before\":1700000000,\"expires_at\":1800000000}}",
            "a".repeat(64)
        )
    }

    fn envelope_with(manifest: &str, signature: &str) -> String {
        serde_json::json!({
            "type": "update.offer",
            "protocol_major": 1,
            "manifest": manifest,
            "signature": signature,
        })
        .to_string()
    }

    fn valid_envelope() -> String {
        envelope_with(&manifest_json(), &signature_der())
    }

    fn temporary_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "bobe-update-offer-{name}-{}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn loader_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let target = temporary_path("target");
        let link = temporary_path("link");
        std::fs::write(&target, valid_envelope()).expect("write offer");
        symlink(&target, &link).expect("create offer symlink");

        let result = UpdateOffer::load(&link);
        std::fs::remove_file(&link).expect("remove link");
        std::fs::remove_file(&target).expect("remove target");

        assert_eq!(
            result,
            Err(OfferError::Read("offer must be a regular file".to_owned()))
        );
    }

    #[test]
    fn valid_envelope_parses_and_preserves_canonical_bytes() {
        let raw = valid_envelope();
        let offer = UpdateOffer::parse(&raw).expect("valid offer");
        assert_eq!(offer.envelope(), raw);
        assert_eq!(offer.build_number(), 3);
        assert_eq!(offer.manifest().version, "0.3.0");
        assert_eq!(offer.manifest().key_id, "prod-2026-01");
        let value: Value = serde_json::from_str(offer.envelope()).unwrap();
        assert_eq!(value["manifest"].as_str().unwrap(), manifest_json());
    }

    #[test]
    fn envelope_rejects_wrong_type_protocol_and_extra_fields() {
        let mut value: Value = serde_json::from_str(&valid_envelope()).unwrap();
        value["type"] = Value::from("update.state");
        assert!(UpdateOffer::parse(&value.to_string()).is_err());

        let mut value: Value = serde_json::from_str(&valid_envelope()).unwrap();
        value["protocol_major"] = Value::from(2);
        assert!(UpdateOffer::parse(&value.to_string()).is_err());

        let mut value: Value = serde_json::from_str(&valid_envelope()).unwrap();
        value["extra"] = Value::from(1);
        assert_eq!(
            UpdateOffer::parse(&value.to_string()),
            Err(OfferError::UnknownField("extra".to_owned()))
        );
    }

    #[test]
    fn envelope_rejects_duplicate_keys() {
        let raw = format!(
            "{{\"type\":\"update.offer\",\"type\":\"update.offer\",\"protocol_major\":1,\
\"manifest\":{},\"signature\":{}}}",
            serde_json::to_string(&manifest_json()).unwrap(),
            serde_json::to_string(&signature_der()).unwrap()
        );
        assert_eq!(
            UpdateOffer::parse(&raw),
            Err(OfferError::DuplicateField("type".to_owned()))
        );
    }

    #[test]
    fn signature_must_be_base64_der_ecdsa() {
        assert!(UpdateOffer::parse(&envelope_with(&manifest_json(), "not base64!!")).is_err());
        assert!(
            UpdateOffer::parse(&envelope_with(
                &manifest_json(),
                &BASE64.encode([0x31, 0x02, 0x01, 0x01]),
            ))
            .is_err()
        );
        assert!(
            UpdateOffer::parse(&envelope_with(
                &manifest_json(),
                &BASE64.encode([0x30, 0x44])
            ))
            .is_err()
        );
    }

    #[test]
    fn manifest_rejects_model_digest_url_and_window_drift() {
        let cases = [
            (
                "\"device_model\":\"wonderboy-wb12\"",
                "\"device_model\":\"other\"",
            ),
            (
                "\"url\":\"https://updates.example/bobe.bin\"",
                "\"url\":\"http://updates.example/bobe.bin\"",
            ),
            ("\"image_size\":2000000", "\"image_size\":6000000"),
            ("\"build_number\":3", "\"build_number\":3.5"),
            ("\"expires_at\":1800000000", "\"expires_at\":1600000000"),
            ("\"schema\":1", "\"schema\":2"),
        ];
        for (from, to) in cases {
            let manifest = manifest_json().replace(from, to);
            assert!(
                UpdateOffer::parse(&envelope_with(&manifest, &signature_der())).is_err(),
                "expected rejection for {to}"
            );
        }

        let manifest = manifest_json().replace(&"a".repeat(64), &"A".repeat(64));
        assert!(UpdateOffer::parse(&envelope_with(&manifest, &signature_der())).is_err());
    }

    #[test]
    fn manifest_rejects_values_the_firmware_cannot_parse() {
        let cases = [
            ("\"version\":\"0.3.0\"", "\"version\":\"v 1\""),
            ("\"version\":\"0.3.0\"", "\"version\":\"é\""),
            (
                "\"expires_at\":1800000000",
                "\"expires_at\":9007199254740992",
            ),
        ];
        for (from, to) in cases {
            let manifest = manifest_json().replace(from, to);
            assert!(
                UpdateOffer::parse(&envelope_with(&manifest, &signature_der())).is_err(),
                "expected rejection for {to}"
            );
        }
    }

    #[test]
    fn manifest_rejects_missing_and_unknown_fields() {
        let manifest = manifest_json().replace("\"key_id\":\"prod-2026-01\",", "");
        assert_eq!(
            UpdateOffer::parse(&envelope_with(&manifest, &signature_der())),
            Err(OfferError::MissingField("key_id"))
        );

        let manifest =
            manifest_json().replace("\"schema\":1,", "\"schema\":1,\"rollout\":\"beta\",");
        assert_eq!(
            UpdateOffer::parse(&envelope_with(&manifest, &signature_der())),
            Err(OfferError::UnknownField("rollout".to_owned()))
        );
    }

    #[test]
    fn envelope_over_the_control_limit_is_rejected() {
        let manifest = manifest_json().replace(
            "https://updates.example/bobe.bin",
            &format!("https://updates.example/{}", "b".repeat(4_000)),
        );
        assert_matches!(
            UpdateOffer::parse(&envelope_with(&manifest, &signature_der())),
            Err(OfferError::TooLarge(_))
        );
    }
}
