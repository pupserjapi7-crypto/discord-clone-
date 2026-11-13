//! Input validation shared by the HTTP API and the gateway.

pub const MAX_HANDLE_LEN: usize = 32;
pub const MAX_DISPLAY_NAME_LEN: usize = 48;
pub const MAX_GUILD_NAME_LEN: usize = 64;
pub const MAX_CHANNEL_NAME_LEN: usize = 48;
pub const MAX_EMOJI_LEN: usize = 32;

pub fn normalize_handle(raw: &str) -> String {
    raw.trim().to_lowercase()
}

pub fn validate_handle(handle: &str) -> Result<(), &'static str> {
    let len = handle.chars().count();
    if !(3..=MAX_HANDLE_LEN).contains(&len) {
        return Err("handle must be 3-32 characters");
    }
    if !handle
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
    {
        return Err("handle may only contain a-z, 0-9, dot and underscore");
    }
    if handle.starts_with('.') || handle.starts_with('_') {
        return Err("handle must start with a letter or digit");
    }
    Ok(())
}

pub fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < 10 {
        return Err("password must be at least 10 characters");
    }
    if password.len() > 256 {
        return Err("password must be at most 256 bytes");
    }
    Ok(())
}

pub fn validate_display_name(name: &str) -> Result<(), &'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("display name must not be empty");
    }
    if trimmed.chars().count() > MAX_DISPLAY_NAME_LEN {
        return Err("display name is too long");
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("display name contains control characters");
    }
    Ok(())
}

pub fn validate_guild_name(name: &str) -> Result<(), &'static str> {
    let trimmed = name.trim();
    if !(2..=MAX_GUILD_NAME_LEN).contains(&trimmed.chars().count()) {
        return Err("guild name must be 2-64 characters");
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("guild name contains control characters");
    }
    Ok(())
}

pub fn validate_channel_name(name: &str) -> Result<(), &'static str> {
    let trimmed = name.trim();
    if !(1..=MAX_CHANNEL_NAME_LEN).contains(&trimmed.chars().count()) {
        return Err("channel name must be 1-48 characters");
    }
    if trimmed.contains(' ') || trimmed.contains('#') || trimmed.chars().any(|c| c.is_control()) {
        return Err("channel name must not contain spaces, '#' or control characters");
    }
    Ok(())
}

/// Message content: non-empty after trimming, bounded, no raw control characters
/// except newline and tab. Length is measured in characters, not bytes.
pub fn validate_message_content(content: &str, max_len: usize) -> Result<(), &'static str> {
    if content.trim().is_empty() {
        return Err("message must not be empty");
    }
    if content.chars().count() > max_len {
        return Err("message is too long");
    }
    if content
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("message contains control characters");
    }
    Ok(())
}

pub fn validate_emoji(emoji: &str) -> Result<(), &'static str> {
    let count = emoji.chars().count();
    if count == 0 || count > MAX_EMOJI_LEN {
        return Err("emoji must be 1-32 characters");
    }
    if emoji.chars().any(|c| c.is_control()) {
        return Err("emoji contains control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_handles() {
        for handle in ["nova", "a.b_c", "user123"] {
            assert!(validate_handle(handle).is_ok(), "{handle}");
        }
    }

    #[test]
    fn rejects_bad_handles() {
        for handle in [
            "",
            "ab",
            "Nova",
            "with space",
            ".hidden",
            "_underscore",
            "ä",
        ] {
            assert!(validate_handle(handle).is_err(), "{handle}");
        }
    }

    #[test]
    fn message_content_rules() {
        assert!(validate_message_content("hi", 8000).is_ok());
        assert!(validate_message_content("   \n ", 8000).is_err());
        assert!(validate_message_content(&"x".repeat(8001), 8000).is_err());
        assert!(validate_message_content("a\u{0}b", 8000).is_err());
        assert!(validate_message_content("line\nbreak\tok", 8000).is_ok());
    }

    #[test]
    fn channel_names_reject_spaces() {
        assert!(validate_channel_name("allgemein").is_ok());
        assert!(validate_channel_name("all gemein").is_err());
    }
}
