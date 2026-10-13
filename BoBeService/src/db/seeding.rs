use tracing::{debug, info, warn};

use crate::db::SqliteSoulRepo;
use crate::db::SqliteUserProfileRepo;
use crate::error::AppError;
use crate::models::soul::Soul;
use crate::models::user_profile::UserProfile;

const DEFAULT_SOULS: &[(&str, &str)] = &[("SOUL.md", "default_soul")];
const DEFAULT_USER_PROFILES: &[(&str, &str)] = &[("USER_PROFILE.md", "default")];

const DEFAULT_SOUL_MD: &str = include_str!("../assets/defaults/SOUL.md");
const DEFAULT_USER_PROFILE_MD: &str = include_str!("../assets/defaults/USER_PROFILE.md");
pub(crate) const DEFAULT_USER_PROFILE_BODY: &str = DEFAULT_USER_PROFILE_MD;

#[derive(Debug)]
pub(crate) struct SeedResult {
    pub(crate) created: u32,
    pub(crate) updated: u32,
    pub(crate) skipped: u32,
    pub(crate) errors: u32,
}

fn load_default_asset(filename: &str) -> Option<&'static str> {
    match filename {
        "SOUL.md" => Some(DEFAULT_SOUL_MD),
        "USER_PROFILE.md" => Some(DEFAULT_USER_PROFILE_MD),
        _ => {
            warn!(filename = filename, "db_seeding.unknown_asset");
            None
        }
    }
}

pub(crate) async fn seed_default_souls(soul_repo: &SqliteSoulRepo) -> Result<SeedResult, AppError> {
    let mut result = SeedResult {
        created: 0,
        updated: 0,
        skipped: 0,
        errors: 0,
    };

    for &(filename, name) in DEFAULT_SOULS {
        let Some(content) = load_default_asset(filename) else {
            warn!(filename = filename, "db_seeding.asset_not_found");
            result.errors += 1;
            continue;
        };

        match soul_repo.get_by_name(name).await {
            Ok(None) => {
                let soul = Soul::new(name.to_owned(), content.to_owned(), true);
                if let Err(e) = soul_repo.save(&soul).await {
                    warn!(name = name, error = %e, "db_seeding.soul_create_failed");
                    result.errors += 1;
                } else {
                    info!(name = name, "db_seeding.soul_created");
                    result.created += 1;
                }
            }
            Ok(Some(existing)) if existing.is_default => {
                if let Err(e) = soul_repo
                    .update(existing.id, Some(content), None, None, None)
                    .await
                {
                    warn!(name = name, error = %e, "db_seeding.soul_update_failed");
                    result.errors += 1;
                } else {
                    debug!(name = name, "db_seeding.soul_updated");
                    result.updated += 1;
                }
            }
            Ok(Some(_)) => {
                debug!(name = name, "db_seeding.soul_skipped_user_modified");
                result.skipped += 1;
            }
            Err(e) => {
                warn!(name = name, error = %e, "db_seeding.soul_check_failed");
                result.errors += 1;
            }
        }
    }

    info!(
        created = result.created,
        updated = result.updated,
        skipped = result.skipped,
        errors = result.errors,
        "db_seeding.souls_complete"
    );
    Ok(result)
}

pub(crate) async fn seed_default_user_profiles(
    profile_repo: &SqliteUserProfileRepo,
) -> Result<SeedResult, AppError> {
    let mut result = SeedResult {
        created: 0,
        updated: 0,
        skipped: 0,
        errors: 0,
    };

    for &(filename, name) in DEFAULT_USER_PROFILES {
        let Some(content) = load_default_asset(filename) else {
            warn!(
                filename = filename,
                "db_seeding.user_profile_asset_not_found"
            );
            result.errors += 1;
            continue;
        };

        match profile_repo.get_by_name(name).await {
            Ok(None) => {
                let profile = UserProfile::new(name.to_owned(), content.to_owned(), true);
                if let Err(e) = profile_repo.save(&profile).await {
                    warn!(name = name, error = %e, "db_seeding.profile_create_failed");
                    result.errors += 1;
                } else {
                    info!(name = name, "db_seeding.user_profile_created");
                    result.created += 1;
                }
            }
            Ok(Some(existing)) if existing.is_default => {
                if let Err(e) = profile_repo.update(existing.id, Some(content), None).await {
                    warn!(name = name, error = %e, "db_seeding.profile_update_failed");
                    result.errors += 1;
                } else {
                    debug!(name = name, "db_seeding.user_profile_updated");
                    result.updated += 1;
                }
            }
            Ok(Some(_)) => {
                debug!(name = name, "db_seeding.user_profile_skipped_user_modified");
                result.skipped += 1;
            }
            Err(e) => {
                warn!(name = name, error = %e, "db_seeding.profile_check_failed");
                result.errors += 1;
            }
        }
    }

    info!(
        created = result.created,
        updated = result.updated,
        skipped = result.skipped,
        errors = result.errors,
        "db_seeding.user_profiles_complete"
    );
    Ok(result)
}
