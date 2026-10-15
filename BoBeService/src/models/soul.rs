use serde::Serialize;

use super::document::{Document, DocumentKind, ShippedDefault};
use super::ids::SoulId;

#[derive(Debug)]
pub(crate) enum Souls {}

impl DocumentKind for Souls {
    type Id = SoulId;
    type Listing = SoulSummary;

    const TABLE: &'static str = "souls";
    const NOUN: &'static str = "Soul";
    const SHIPPED_DEFAULT: Option<ShippedDefault> = Some(ShippedDefault {
        name: "default_soul",
        content: include_str!("../assets/defaults/SOUL.md"),
    });

    fn new_id() -> SoulId {
        SoulId::new()
    }
}

pub(crate) type Soul = Document<Souls>;

#[derive(Debug, Serialize)]
pub(crate) struct SoulSummary {
    pub(crate) souls: Vec<Soul>,
    pub(crate) count: usize,
    pub(crate) enabled_count: usize,
}

impl From<Vec<Soul>> for SoulSummary {
    fn from(souls: Vec<Soul>) -> Self {
        Self {
            count: souls.len(),
            enabled_count: souls.iter().filter(|soul| soul.enabled).count(),
            souls,
        }
    }
}
