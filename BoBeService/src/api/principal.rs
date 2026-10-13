use super::scope::ScopeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrincipalKind {
    Owner,
    Endpoint,
    Adapter,
}

#[derive(Clone, Debug)]
pub(crate) struct Principal {
    pub(crate) kind: PrincipalKind,
    pub(crate) endpoint_id: Option<String>,
    pub(crate) credential_id: Option<String>,
    pub(crate) issued_authority_epoch: Option<i64>,
    pub(crate) scopes: ScopeSet,
}

impl Principal {
    pub(crate) fn conversation_channel(&self) -> crate::models::ids::ConversationChannelId {
        self.endpoint_id.as_deref().map_or(
            crate::models::ids::ConversationChannelId::OWNER,
            crate::models::ids::ConversationChannelId::for_endpoint,
        )
    }

    pub(crate) fn owner() -> Self {
        Self {
            kind: PrincipalKind::Owner,
            endpoint_id: None,
            credential_id: None,
            issued_authority_epoch: None,
            scopes: ScopeSet::all(),
        }
    }

    pub(crate) fn adapter() -> Self {
        Self {
            kind: PrincipalKind::Adapter,
            endpoint_id: None,
            credential_id: None,
            issued_authority_epoch: None,
            scopes: ScopeSet::from_static(&[
                super::scope::Scope::VoiceSession,
                super::scope::Scope::CaptureSubmit,
            ]),
        }
    }

    pub(crate) fn endpoint(
        endpoint_id: String,
        credential_id: String,
        issued_authority_epoch: i64,
        scopes: ScopeSet,
    ) -> Self {
        Self {
            kind: PrincipalKind::Endpoint,
            endpoint_id: Some(endpoint_id),
            credential_id: Some(credential_id),
            issued_authority_epoch: Some(issued_authority_epoch),
            scopes,
        }
    }
}
