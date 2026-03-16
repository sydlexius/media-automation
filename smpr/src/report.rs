// Remove this allow after wiring report output in Task 10.
#![allow(dead_code)]

use crate::rating::ItemResult;
use std::path::Path;

/// Write detection results to a CSV file.
///
/// Creates parent directories if needed. Errors are logged, not fatal.
pub fn write_report(results: &[ItemResult], path: &Path) {
    if let Err(e) = write_report_inner(results, path) {
        log::error!("cannot write report to {}: {}", path.display(), e);
    } else {
        log::info!("report written to {}", path.display());
    }
}

fn write_report_inner(
    results: &[ItemResult],
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record([
        "artist",
        "album",
        "track",
        "tier",
        "matched_words",
        "previous_rating",
        "action",
        "source",
        "server",
    ])?;
    for r in results {
        let track = r
            .path
            .as_deref()
            .and_then(|p| p.rsplit(['/', '\\']).next())
            .unwrap_or("");
        wtr.write_record([
            r.artist.as_deref().unwrap_or(""),
            r.album.as_deref().unwrap_or(""),
            track,
            r.tier.as_deref().unwrap_or(""),
            &r.matched_words.join("; "),
            r.previous_rating.as_deref().unwrap_or(""),
            r.action.as_csv_str(),
            r.source.as_csv_str(),
            &r.server_name,
        ])?;
    }
    wtr.flush()?;
    Ok(())
}
