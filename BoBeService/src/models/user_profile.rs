use serde::Serialize;

use super::document::{Document, DocumentKind, ShippedDefault};
use super::ids::UserProfileId;

#[derive(Debug)]
pub(crate) enum UserProfiles {}

impl DocumentKind for UserProfiles {
    type Id = UserProfileId;
    type Listing = UserProfileSummary;

    const TABLE: &'static str = "user_profiles";
    const NOUN: &'static str = "User profile";
    const SHIPPED_DEFAULT: Option<ShippedDefault> = None;

    fn new_id() -> UserProfileId {
        UserProfileId::new()
    }
}

pub(crate) type UserProfile = Document<UserProfiles>;

#[derive(Debug, Serialize)]
pub(crate) struct UserProfileSummary {
    pub(crate) profiles: Vec<UserProfile>,
    pub(crate) count: usize,
    pub(crate) enabled_count: usize,
}

impl From<Vec<UserProfile>> for UserProfileSummary {
    fn from(profiles: Vec<UserProfile>) -> Self {
        Self {
            count: profiles.len(),
            enabled_count: profiles.iter().filter(|profile| profile.enabled).count(),
            profiles,
        }
    }
}
