use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("frame is shorter than the {expected}-byte header")]
    TooShort { expected: usize },
    #[error("frame body is {size} bytes, maximum is {max}")]
    TooLarge { size: usize, max: usize },
    #[error("frame declares {declared} body bytes but carries {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("messagepack encode failed: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("messagepack decode failed: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("unknown opcode {0}")]
    UnknownOp(u8),
}
