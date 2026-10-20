use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            serde::Serialize,
            serde::Deserialize,
            sqlx::Type,
        )]
        #[serde(transparent)]
        #[sqlx(transparent)]
        pub(crate) struct $name(Uuid);

        impl $name {
            pub(crate) fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::from_str(s).map(Self)
            }
        }
    };
}

define_id!(ConversationId);
define_id!(ConversationTurnId);
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub(crate) struct ConversationChannelId(Uuid);

impl fmt::Display for ConversationChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
define_id!(GoalId);
define_id!(SoulId);
define_id!(UserProfileId);
define_id!(CooldownId);

impl ConversationChannelId {
    pub(crate) const OWNER: Self = Self(Uuid::nil());

    pub(crate) fn for_endpoint(endpoint_id: &str) -> Self {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(format!("bobe-channel:{endpoint_id}").as_bytes());
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self(Uuid::from_bytes(bytes))
    }
}

impl FromStr for ConversationChannelId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

impl Default for ConversationChannelId {
    fn default() -> Self {
        Self::OWNER
    }
}

pub(crate) fn new_message_id() -> String {
    message_id_for_turn(ConversationTurnId::new())
}

pub(crate) fn message_id_for_turn(turn_id: ConversationTurnId) -> String {
    let uuid = Uuid::from(turn_id);
    format!("msg_{}", uuid.simple())
}

pub(crate) fn conversation_turn_id_from_wire_id(
    wire_id: &str,
) -> Result<ConversationTurnId, uuid::Error> {
    let uuid_suffix = wire_id.rsplit('_').next().unwrap_or(wire_id);
    Uuid::parse_str(uuid_suffix).map(ConversationTurnId::from)
}

pub(crate) fn new_turn_id() -> String {
    format!("voice_{}", Uuid::from(ConversationTurnId::new()).simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newtype_round_trips_through_uuid() {
        let id = ConversationId::new();
        let uuid: Uuid = id.into();
        let back = ConversationId::from(uuid);
        assert_eq!(id, back);
    }

    #[test]
    fn newtype_display_matches_uuid() {
        let uuid = Uuid::new_v4();
        let id = GoalId::from(uuid);
        assert_eq!(id.to_string(), uuid.to_string());
    }

    #[test]
    fn newtype_parse_from_string() {
        let uuid = Uuid::new_v4();
        let s = uuid.to_string();
        let id: GoalId = s.parse().unwrap();
        assert_eq!(Uuid::from(id), uuid);
    }

    #[test]
    fn newtype_serde_transparent() {
        let id = SoulId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: SoulId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
        assert_eq!(json, format!("\"{id}\""));
    }

    #[test]
    fn wire_message_id_round_trips_to_conversation_turn_id() {
        let turn_id = ConversationTurnId::new();
        let wire_id = message_id_for_turn(turn_id);

        let parsed = conversation_turn_id_from_wire_id(&wire_id).expect("valid message id");

        assert_eq!(parsed, turn_id);
    }

    #[test]
    fn voice_wire_id_maps_to_conversation_turn_id() {
        let wire_id = new_turn_id();
        let parsed = conversation_turn_id_from_wire_id(&wire_id).expect("valid voice turn id");
        let uuid = Uuid::from(parsed);

        assert!(wire_id.ends_with(&uuid.simple().to_string()));
    }
}
