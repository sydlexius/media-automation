use super::WizardError;
use crate::config::ServerType;

/// Result of the server connection step.
pub struct ServerInfo {
    pub url: String,
    pub label: String,
    pub server_type: ServerType,
}

pub fn prompt_server(_verbose: bool) -> Result<ServerInfo, WizardError> {
    todo!("Task 9")
}
