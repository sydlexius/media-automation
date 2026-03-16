use crate::rating::RatingAction;
use crate::server::MediaServerError;

/// Decide what action to take for setting a rating.
///
/// Pure logic — no server calls. Returns the action to take.
/// When `Set` is returned, the caller must perform the server round-trip.
pub fn decide_rating_action(
    tier: &str,
    current_rating: Option<&str>,
    overwrite: bool,
    dry_run: bool,
) -> RatingAction {
    // Already at the desired rating?
    if current_rating.is_some_and(|r| r == tier) {
        return RatingAction::AlreadyCorrect;
    }
    // Skip if has existing rating and overwrite is false
    if !overwrite && current_rating.is_some_and(|r| !r.is_empty()) {
        return RatingAction::Skipped;
    }
    if dry_run {
        return RatingAction::DryRun;
    }
    RatingAction::Set
}

/// Decide what action to take for clearing a rating.
///
/// Used when lyrics are clean but a track has an existing rating (overwrite mode).
pub fn decide_clear_action(
    current_rating: Option<&str>,
    overwrite: bool,
    dry_run: bool,
) -> RatingAction {
    // No rating to clear
    if current_rating.is_none() || current_rating.is_some_and(|r| r.is_empty()) {
        return RatingAction::Skipped;
    }
    // Skip-existing mode: don't touch rated tracks
    if !overwrite {
        return RatingAction::Skipped;
    }
    if dry_run {
        return RatingAction::DryRunClear;
    }
    RatingAction::Cleared
}

/// GET-then-POST round-trip to set OfficialRating on an item.
/// Returns the final `RatingAction` (Set, Cleared, or Error).
pub fn apply_rating(
    client: &crate::server::MediaServerClient,
    item_id: &str,
    rating: &str,
    label: &str,
) -> RatingAction {
    match apply_rating_inner(client, item_id, rating) {
        Ok(()) => {
            if rating.is_empty() {
                log::info!("cleared rating from {}", label);
                RatingAction::Cleared
            } else {
                log::info!("set {} on {}", rating, label);
                RatingAction::Set
            }
        }
        Err(e) => {
            log::error!("failed to update {}: {}", label, e);
            RatingAction::Error(e.to_string())
        }
    }
}

fn apply_rating_inner(
    client: &crate::server::MediaServerClient,
    item_id: &str,
    rating: &str,
) -> Result<(), MediaServerError> {
    let mut item = client.get_item(item_id)?;
    item["OfficialRating"] = serde_json::Value::String(rating.to_string());
    client.update_item(item_id, &item)?;
    Ok(())
}
