use super::WizardError;

/// Result of the detection rules step.
pub struct DetectionAdditions {
    pub extra_r_stems: Vec<String>,
    pub extra_r_exact: Vec<String>,
    pub extra_pg13_stems: Vec<String>,
    pub extra_pg13_exact: Vec<String>,
    pub extra_false_positives: Vec<String>,
}

pub fn prompt_detection(_verbose: bool) -> Result<DetectionAdditions, WizardError> {
    todo!("Task 12")
}
