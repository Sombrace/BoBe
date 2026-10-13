use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::Utc;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair, PublicKeyData,
    SanType,
};
use rustls::pki_types::pem::{PemObject, SectionKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::FromDer;

use crate::error::AppError;
use crate::util::paths::DataPaths;
use crate::util::text::hex_encode;

const CERTIFICATE_VALIDITY: Duration = Duration::from_hours(397 * 24);
const RENEWAL_WINDOW: Duration = Duration::from_hours(30 * 24);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AutomaticTlsIdentity {
    pub(crate) certificate_path: PathBuf,
    pub(crate) private_key_path: PathBuf,
    pub(crate) spki_sha256: String,
    pub(crate) hosts: Vec<String>,
    pub(crate) generated_at: chrono::DateTime<Utc>,
}

struct CertificateFacts {
    spki: Vec<u8>,
    hosts: Vec<String>,
    not_after: SystemTime,
}

pub(crate) async fn ensure(
    paths: &DataPaths,
    configured_hosts: &[String],
) -> Result<AutomaticTlsIdentity, AppError> {
    let hosts = normalize_hosts(configured_hosts)?;
    let certificate_path = paths.automatic_tls_certificate();
    let private_key_path = paths.automatic_tls_private_key();
    let metadata_path = paths.automatic_tls_metadata();
    let has_certificate = certificate_path.exists();
    let has_private_key = private_key_path.exists();
    let has_metadata = metadata_path.exists();

    if has_certificate || has_private_key || has_metadata {
        if !(certificate_path.is_file() && private_key_path.is_file() && metadata_path.is_file()) {
            return Err(AppError::Config(
                "automatic TLS identity is incomplete; restore all identity files or explicitly reset remote trust"
                    .into(),
            ));
        }
        let key_pem = tokio::fs::read_to_string(&private_key_path).await?;
        let key_pair = KeyPair::from_pem(&key_pem).map_err(|error| {
            AppError::Config(format!("parse automatic TLS private key: {error}"))
        })?;
        let certificate_pem = tokio::fs::read(&certificate_path).await?;
        let facts = certificate_facts(&certificate_pem)?;
        if facts.spki != key_pair.subject_public_key_info() {
            return Err(AppError::Config(
                "automatic TLS certificate does not match its private key; restore the matching identity or explicitly reset remote trust"
                    .into(),
            ));
        }
        let metadata: AutomaticTlsIdentity =
            serde_json::from_slice(&tokio::fs::read(&metadata_path).await?)?;
        let spki_sha256 = hex_encode(&Sha256::digest(&facts.spki));
        if metadata.spki_sha256 != spki_sha256 {
            return Err(AppError::Config(
                "automatic TLS metadata does not match the certificate; restore all identity files or explicitly reset remote trust"
                    .into(),
            ));
        }
        let renew_before = SystemTime::now()
            .checked_add(RENEWAL_WINDOW)
            .ok_or_else(|| AppError::Internal("calculate TLS renewal window".into()))?;
        if metadata.hosts != hosts || facts.hosts != hosts || facts.not_after <= renew_before {
            return write_identity(
                paths,
                key_pair,
                hosts,
                certificate_path,
                private_key_path,
                metadata_path,
                false,
            )
            .await;
        }
        return Ok(AutomaticTlsIdentity {
            certificate_path,
            private_key_path,
            spki_sha256,
            hosts,
            generated_at: metadata.generated_at,
        });
    }

    prepare_tls_directory(paths).await?;
    let key_pair = KeyPair::generate()
        .map_err(|error| AppError::Internal(format!("generate TLS key: {error}")))?;
    write_identity(
        paths,
        key_pair,
        hosts,
        certificate_path,
        private_key_path,
        metadata_path,
        true,
    )
    .await
}

async fn write_identity(
    paths: &DataPaths,
    key_pair: KeyPair,
    hosts: Vec<String>,
    certificate_path: PathBuf,
    private_key_path: PathBuf,
    metadata_path: PathBuf,
    write_private_key: bool,
) -> Result<AutomaticTlsIdentity, AppError> {
    prepare_tls_directory(paths).await?;
    let certificate_pem =
        issue_certificate(&key_pair, &hosts, SystemTime::now(), CERTIFICATE_VALIDITY)?;
    let spki_sha256 = spki_sha256_from_certificate_pem(certificate_pem.as_bytes())?;
    if write_private_key {
        crate::util::durable_fs::atomic_write_mode(
            &private_key_path,
            key_pair.serialize_pem().as_bytes(),
            0o600,
        )
        .await?;
    }
    crate::util::durable_fs::atomic_write_mode(
        &certificate_path,
        certificate_pem.as_bytes(),
        0o600,
    )
    .await?;
    let identity = AutomaticTlsIdentity {
        certificate_path,
        private_key_path,
        spki_sha256,
        hosts,
        generated_at: Utc::now(),
    };
    crate::util::durable_fs::atomic_write_mode(
        &metadata_path,
        &serde_json::to_vec_pretty(&identity)?,
        0o600,
    )
    .await?;
    Ok(identity)
}

fn issue_certificate(
    key_pair: &KeyPair,
    hosts: &[String],
    not_before: SystemTime,
    validity: Duration,
) -> Result<String, AppError> {
    let mut params = CertificateParams::new(hosts.to_vec())
        .map_err(|error| AppError::Config(format!("invalid TLS host: {error}")))?;
    params.not_before = not_before.into();
    params.not_after = not_before
        .checked_add(validity)
        .ok_or_else(|| AppError::Internal("calculate TLS certificate expiry".into()))?
        .into();
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, "BoBe personal runtime");
    params.distinguished_name = name;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.subject_alt_names = hosts
        .iter()
        .map(|host| {
            if let Ok(address) = host.parse::<IpAddr>() {
                Ok(SanType::IpAddress(address))
            } else {
                let name = host.clone().try_into().map_err(|error| {
                    AppError::Config(format!("invalid TLS DNS name {host}: {error}"))
                })?;
                Ok(SanType::DnsName(name))
            }
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    params
        .self_signed(key_pair)
        .map(|certificate| certificate.pem())
        .map_err(|error| AppError::Internal(format!("sign TLS certificate: {error}")))
}

async fn prepare_tls_directory(paths: &DataPaths) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(paths.tls_dir())?;
        std::fs::set_permissions(paths.tls_dir(), std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    tokio::fs::create_dir_all(paths.tls_dir()).await?;
    Ok(())
}

fn normalize_hosts(configured_hosts: &[String]) -> Result<Vec<String>, AppError> {
    let mut hosts = configured_hosts
        .iter()
        .map(|host| host.trim().to_lowercase())
        .filter(|host| !host.is_empty())
        .collect::<Vec<_>>();
    hosts.sort();
    hosts.dedup();
    if hosts.is_empty() {
        return Err(AppError::Config(
            "automatic TLS requires at least one allowed host or IP address".into(),
        ));
    }
    hosts.extend(["127.0.0.1", "::1", "localhost"].map(str::to_owned));
    hosts.sort();
    hosts.dedup();
    Ok(hosts)
}

fn certificate_facts(pem_value: &[u8]) -> Result<CertificateFacts, AppError> {
    let sections = <(SectionKind, Vec<u8>)>::pem_slice_iter(pem_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Config(format!("parse TLS certificate PEM: {error}")))?;
    let [(SectionKind::Certificate, certificate_der)] = sections.as_slice() else {
        return Err(AppError::Config(
            "automatic TLS certificate PEM must contain exactly one certificate".into(),
        ));
    };
    let (_, certificate) = x509_parser::certificate::X509Certificate::from_der(certificate_der)
        .map_err(|error| AppError::Config(format!("parse TLS certificate: {error}")))?;
    let alternative_names = certificate
        .subject_alternative_name()
        .map_err(|error| AppError::Config(format!("parse TLS certificate SANs: {error}")))?
        .ok_or_else(|| AppError::Config("automatic TLS certificate has no SAN extension".into()))?;
    let mut hosts = alternative_names
        .value
        .general_names
        .iter()
        .map(|name| match name {
            GeneralName::DNSName(name) => Ok((*name).to_lowercase()),
            GeneralName::IPAddress(bytes) => match bytes.len() {
                4 => Ok(
                    IpAddr::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])).to_string(),
                ),
                16 => {
                    let bytes: [u8; 16] = (*bytes).try_into().map_err(|_| {
                        AppError::Config("invalid IPv6 SAN in automatic TLS certificate".into())
                    })?;
                    Ok(IpAddr::V6(Ipv6Addr::from(bytes)).to_string())
                }
                _ => Err(AppError::Config(
                    "invalid IP SAN in automatic TLS certificate".into(),
                )),
            },
            _ => Err(AppError::Config(
                "automatic TLS certificate contains an unsupported SAN type".into(),
            )),
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    hosts.sort();
    hosts.dedup();
    let expiry = certificate.validity().not_after.timestamp();
    let not_after = if expiry >= 0 {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(expiry as u64))
            .ok_or_else(|| AppError::Config("invalid automatic TLS certificate expiry".into()))?
    } else {
        return Err(AppError::Config(
            "automatic TLS certificate has an invalid expiry".into(),
        ));
    };
    Ok(CertificateFacts {
        spki: certificate.tbs_certificate.subject_pki.raw.to_vec(),
        hosts,
        not_after,
    })
}

pub(crate) fn spki_sha256_from_certificate_pem(pem_value: &[u8]) -> Result<String, AppError> {
    Ok(hex_encode(&Sha256::digest(
        certificate_facts(pem_value)?.spki,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (PathBuf, DataPaths) {
        let root = std::env::temp_dir().join(format!("bobe-tls-{}", uuid::Uuid::new_v4()));
        let paths = DataPaths::resolve(&root, &format!("sqlite:{}/data.db", root.display()))
            .unwrap()
            .prepare_root()
            .unwrap();
        (root, paths)
    }

    #[tokio::test]
    async fn host_changes_renew_certificate_without_changing_spki() {
        let (root, paths) = paths();
        let first = ensure(&paths, &["192.168.1.5".into()]).await.unwrap();
        let first_certificate = tokio::fs::read(&first.certificate_path).await.unwrap();
        let second = ensure(&paths, &["192.168.1.6".into()]).await.unwrap();
        let second_certificate = tokio::fs::read(&second.certificate_path).await.unwrap();
        assert_eq!(first.spki_sha256, second.spki_sha256);
        assert_ne!(first_certificate, second_certificate);
        assert_eq!(
            certificate_facts(&second_certificate).unwrap().hosts,
            vec!["127.0.0.1", "192.168.1.6", "::1", "localhost"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn near_expiry_renews_with_same_key() {
        let (root, paths) = paths();
        let first = ensure(&paths, &["bobe.example".into()]).await.unwrap();
        let key = KeyPair::from_pem(
            &tokio::fs::read_to_string(&first.private_key_path)
                .await
                .unwrap(),
        )
        .unwrap();
        let expiring = issue_certificate(
            &key,
            &["bobe.example".into()],
            SystemTime::now(),
            Duration::from_hours(24),
        )
        .unwrap();
        crate::util::durable_fs::atomic_write_mode(
            &first.certificate_path,
            expiring.as_bytes(),
            0o600,
        )
        .await
        .unwrap();
        let renewed = ensure(&paths, &["bobe.example".into()]).await.unwrap();
        assert_eq!(first.spki_sha256, renewed.spki_sha256);
        assert!(
            certificate_facts(&tokio::fs::read(&renewed.certificate_path).await.unwrap())
                .unwrap()
                .not_after
                > SystemTime::now() + RENEWAL_WINDOW
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn mismatched_private_key_fails_closed() {
        let (root, paths) = paths();
        ensure(&paths, &["bobe.example".into()]).await.unwrap();
        let replacement = KeyPair::generate().unwrap();
        crate::util::durable_fs::atomic_write_mode(
            &paths.automatic_tls_private_key(),
            replacement.serialize_pem().as_bytes(),
            0o600,
        )
        .await
        .unwrap();
        let error = ensure(&paths, &["bobe.example".into()])
            .await
            .expect_err("mismatched identity must fail");
        assert!(error.to_string().contains("does not match its private key"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn creates_private_stable_identity_for_ip_and_hostname() {
        let (root, paths) = paths();
        let first = ensure(&paths, &["bobe.example".into(), "192.168.1.5".into()])
            .await
            .unwrap();
        let second = ensure(&paths, &["192.168.1.5".into(), "bobe.example".into()])
            .await
            .unwrap();
        assert_eq!(first.spki_sha256, second.spki_sha256);
        assert_eq!(first.spki_sha256.len(), 64);
        assert!(first.certificate_path.is_file());
        assert!(first.private_key_path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&first.private_key_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
