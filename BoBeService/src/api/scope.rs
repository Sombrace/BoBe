use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scope {
    ConversationRead,
    ConversationWrite,
    MemoryRead,
    MemoryWrite,
    GoalsRead,
    GoalsWrite,
    SettingsRead,
    SettingsWrite,
    ProviderAdmin,
    VoiceSession,
    CaptureSubmit,
    BodyAdmin,
    EventsSubscribe,
}

impl Scope {
    pub(crate) const ALL: [Self; 13] = [
        Self::ConversationRead,
        Self::ConversationWrite,
        Self::MemoryRead,
        Self::MemoryWrite,
        Self::GoalsRead,
        Self::GoalsWrite,
        Self::SettingsRead,
        Self::SettingsWrite,
        Self::ProviderAdmin,
        Self::VoiceSession,
        Self::CaptureSubmit,
        Self::BodyAdmin,
        Self::EventsSubscribe,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ConversationRead => "conversation_read",
            Self::ConversationWrite => "conversation_write",
            Self::MemoryRead => "memory_read",
            Self::MemoryWrite => "memory_write",
            Self::GoalsRead => "goals_read",
            Self::GoalsWrite => "goals_write",
            Self::SettingsRead => "settings_read",
            Self::SettingsWrite => "settings_write",
            Self::ProviderAdmin => "provider_admin",
            Self::VoiceSession => "voice_session",
            Self::CaptureSubmit => "capture_submit",
            Self::BodyAdmin => "body_admin",
            Self::EventsSubscribe => "events_subscribe",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScopeProfile {
    MacApp,
    Phone,
}

impl ScopeProfile {
    const PHONE: [Scope; 9] = [
        Scope::ConversationRead,
        Scope::ConversationWrite,
        Scope::MemoryRead,
        Scope::MemoryWrite,
        Scope::GoalsRead,
        Scope::GoalsWrite,
        Scope::SettingsRead,
        Scope::VoiceSession,
        Scope::EventsSubscribe,
    ];

    pub(crate) fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "mac_app" => Some(Self::MacApp),
            "phone" => Some(Self::Phone),
            _ => None,
        }
    }

    pub(crate) const fn kind(self) -> &'static str {
        match self {
            Self::MacApp => "mac_app",
            Self::Phone => "phone",
        }
    }

    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::MacApp => "mac_app.v1",
            Self::Phone => "phone.v1",
        }
    }

    pub(crate) const fn endpoint_kind(self) -> &'static str {
        match self {
            Self::MacApp => "mac_app",
            Self::Phone => "other",
        }
    }

    pub(crate) const fn default_label(self) -> &'static str {
        match self {
            Self::MacApp => "Mac",
            Self::Phone => "iPhone",
        }
    }

    pub(crate) const fn scopes(self) -> &'static [Scope] {
        match self {
            Self::MacApp => &Scope::ALL,
            Self::Phone => &Self::PHONE,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScopeSet {
    scopes: Vec<Scope>,
}

impl ScopeSet {
    pub(crate) fn all() -> Self {
        Self {
            scopes: Scope::ALL.to_vec(),
        }
    }

    pub(crate) fn from_static(scopes: &'static [Scope]) -> Self {
        Self {
            scopes: scopes.to_vec(),
        }
    }

    pub(crate) fn from_iter(scopes: impl IntoIterator<Item = Scope>) -> Self {
        let mut scopes: Vec<_> = scopes.into_iter().collect();
        scopes.sort_unstable_by_key(|scope| scope.as_str());
        scopes.dedup();
        Self { scopes }
    }

    pub(crate) fn contains(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }

    pub(crate) fn grant_requested(&self, requested: Vec<Scope>) -> Vec<Scope> {
        requested
            .into_iter()
            .filter(|scope| self.contains(*scope))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_principal_only_grants_its_authenticated_scopes() {
        let scopes = ScopeSet::from_static(&[Scope::VoiceSession, Scope::CaptureSubmit]);
        assert_eq!(
            scopes.grant_requested(vec![
                Scope::ProviderAdmin,
                Scope::VoiceSession,
                Scope::CaptureSubmit,
            ]),
            vec![Scope::VoiceSession, Scope::CaptureSubmit]
        );
    }

    #[test]
    fn a_phone_profile_leaves_out_every_administrative_scope() {
        let phone = ScopeSet::from_static(ScopeProfile::Phone.scopes());
        for scope in [
            Scope::SettingsWrite,
            Scope::ProviderAdmin,
            Scope::CaptureSubmit,
            Scope::BodyAdmin,
        ] {
            assert!(!phone.contains(scope), "{scope:?}");
        }
        assert_eq!(ScopeProfile::Phone.scopes().len(), 9);
        assert_eq!(ScopeProfile::MacApp.scopes(), Scope::ALL.as_slice());
        assert_eq!(ScopeProfile::from_kind("phone"), Some(ScopeProfile::Phone));
        assert_eq!(ScopeProfile::Phone.endpoint_kind(), "other");
        assert_eq!(ScopeProfile::from_kind("other"), None);
    }
}
