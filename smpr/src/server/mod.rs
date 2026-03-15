// Server module methods are consumed by future milestones (detection, rating).
#![allow(dead_code, unused_imports)]

pub mod error;
pub mod types;

#[cfg(test)]
mod tests;

pub use error::MediaServerError;
