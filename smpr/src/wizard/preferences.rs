use super::WizardError;

/// Result of the preferences step.
pub struct Preferences {
    pub overwrite: bool,
}

pub fn prompt_preferences() -> Result<Preferences, WizardError> {
    todo!("Task 13")
}
