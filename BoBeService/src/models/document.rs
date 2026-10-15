use std::fmt;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::Sqlite;

pub(crate) struct ShippedDefault {
    pub(crate) name: &'static str,
    pub(crate) content: &'static str,
}

pub(crate) trait DocumentKind: Sized + Send + Sync + 'static {
    type Id: Copy
        + Eq
        + fmt::Debug
        + fmt::Display
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + Unpin
        + 'static
        + sqlx::Type<Sqlite>
        + for<'q> sqlx::Encode<'q, Sqlite>
        + for<'r> sqlx::Decode<'r, Sqlite>;
    type Listing: From<Vec<Document<Self>>> + Serialize + Send;

    const TABLE: &'static str;
    const NOUN: &'static str;
    const SHIPPED_DEFAULT: Option<ShippedDefault>;

    fn new_id() -> Self::Id;
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub(crate) struct Document<K: DocumentKind> {
    pub(crate) id: K::Id,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) enabled: bool,
    pub(crate) is_default: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl<K: DocumentKind> Document<K> {
    pub(crate) fn new(name: String, content: String, enabled: bool, is_default: bool) -> Self {
        let now = Utc::now();
        Self {
            id: K::new_id(),
            name,
            content,
            enabled,
            is_default,
            created_at: now,
            updated_at: now,
        }
    }

    pub(crate) fn is_shipped_default(&self) -> bool {
        K::SHIPPED_DEFAULT.is_some() && self.is_default
    }
}
