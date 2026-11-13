//! Voice/SFU seam.
//!
//! Milestone 1 ships no media path, but the boundary exists already so the
//! gateway and the UI never depend on a concrete SFU implementation. Milestone 2
//! adds a `str0m` backend behind [`VoiceBackend`]; if the latency gate fails, a
//! LiveKit adapter can implement the same trait.

use nova_common::protocol::UserPublic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceError {
    NotImplemented,
    Unavailable,
    NotPermitted,
}

pub enum Signal {
    Offer(String),
    Answer(String),
    IceCandidate(String),
}

/// A media backend for one voice channel.
pub trait VoiceBackend: Send + Sync {
    fn join(&self, channel_id: i64, user: &UserPublic) -> Result<Signal, VoiceError>;
    fn signal(&self, channel_id: i64, user_id: i64, signal: Signal) -> Result<(), VoiceError>;
    fn leave(&self, channel_id: i64, user_id: i64);
}

/// Placeholder used until milestone 2.
#[derive(Debug, Default)]
pub struct UnavailableBackend;

impl VoiceBackend for UnavailableBackend {
    fn join(&self, _channel_id: i64, _user: &UserPublic) -> Result<Signal, VoiceError> {
        Err(VoiceError::NotImplemented)
    }

    fn signal(&self, _channel_id: i64, _user_id: i64, _signal: Signal) -> Result<(), VoiceError> {
        Err(VoiceError::NotImplemented)
    }

    fn leave(&self, _channel_id: i64, _user_id: i64) {}
}
