use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::db::device_trust_repo::DeviceTrustRepo;
use crate::db::endpoint_repo::EndpointRecord;
use crate::error::AppError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BodyPeerIdentity {
    pub(crate) endpoint_id: String,
    pub(crate) certificate_sha256: [u8; 32],
}

#[derive(Default)]
struct IdentitySnapshot {
    by_certificate: HashMap<[u8; 32], BodyPeerIdentity>,
}

pub(crate) struct BodyIdentityIndex {
    snapshot: ArcSwap<IdentitySnapshot>,
}

impl BodyIdentityIndex {
    pub(crate) async fn load(repo: &DeviceTrustRepo) -> Result<Arc<Self>, AppError> {
        let index = Arc::new(Self {
            snapshot: ArcSwap::from_pointee(IdentitySnapshot::default()),
        });
        index.refresh(repo).await?;
        Ok(index)
    }

    pub(crate) async fn refresh(&self, repo: &DeviceTrustRepo) -> Result<(), AppError> {
        let mut snapshot = IdentitySnapshot::default();
        for (endpoint, certificate) in repo.active_body_credentials().await? {
            let Some(identity) = identity_from_record(&endpoint, &certificate) else {
                continue;
            };
            snapshot
                .by_certificate
                .insert(identity.certificate_sha256, identity);
        }
        self.snapshot.store(Arc::new(snapshot));
        Ok(())
    }

    pub(crate) fn by_certificate(&self, digest: &[u8; 32]) -> Option<BodyPeerIdentity> {
        self.snapshot.load().by_certificate.get(digest).cloned()
    }
}

fn identity_from_record(
    endpoint: &EndpointRecord,
    certificate_sha256: &[u8],
) -> Option<BodyPeerIdentity> {
    if endpoint.kind != "physical_body" {
        return None;
    }
    let digest = *certificate_sha256.as_array()?;
    Some(BodyPeerIdentity {
        endpoint_id: endpoint.id.clone(),
        certificate_sha256: digest,
    })
}
