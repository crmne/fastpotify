//! Spotify Web API client.

use std::fmt;

pub mod client;
pub mod gateway;
pub mod models;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApiSource {
    Shared,
    Personal,
}

impl fmt::Display for ApiSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shared => formatter.write_str("shared"),
            Self::Personal => formatter.write_str("personal"),
        }
    }
}

pub use client::{ApiClient, ApiError, NetActivity, PlayRequest, TokenProvider, WebTokens};
pub use gateway::{AccountId, ApiGateway, Operation, PlaylistAccess, PlaylistId, SessionState};
