use super::{WizardError, from_inquire};

/// Result of the preferences step.
pub struct Preferences {
    pub overwrite: bool,
}

pub fn prompt_preferences() -> Result<Preferences, WizardError> {
    println!("\n── Preferences ──\n");

    let options = vec![
        "Overwrite — re-evaluate all tracks, update ratings as needed",
        "Skip — leave tracks that already have a rating",
    ];
    let choice = inquire::Select::new("When a track already has a rating:", options)
        .prompt()
        .map_err(from_inquire)?;

    let overwrite = choice.starts_with("Overwrite");

    Ok(Preferences { overwrite })
}
