use serde::{Deserialize, Serialize};

bitflags::bitflags! {
    /// Guild-level permission bitfield. Stored as `BIGINT` in PostgreSQL.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct Permissions: u64 {
        const ADMINISTRATOR      = 1 << 0;
        const MANAGE_GUILD       = 1 << 1;
        const MANAGE_ROLES       = 1 << 2;
        const MANAGE_CHANNELS    = 1 << 3;
        const KICK_MEMBERS       = 1 << 4;
        const BAN_MEMBERS        = 1 << 5;
        const TIMEOUT_MEMBERS    = 1 << 6;
        const VIEW_CHANNEL       = 1 << 7;
        const SEND_MESSAGES      = 1 << 8;
        const MANAGE_MESSAGES    = 1 << 9;
        const ADD_REACTIONS      = 1 << 10;
        const ATTACH_FILES       = 1 << 11;
        const READ_MESSAGE_HISTORY = 1 << 12;
        const CONNECT            = 1 << 13;
        const SPEAK              = 1 << 14;
        const STREAM             = 1 << 15;
        const MUTE_MEMBERS       = 1 << 16;
        const DEAFEN_MEMBERS     = 1 << 17;
        const MOVE_MEMBERS       = 1 << 18;
        const CREATE_INVITE      = 1 << 19;
        const MANAGE_WEBHOOKS    = 1 << 20;
        const SEND_VOICE_MESSAGES = 1 << 21;
        const USE_SOUNDBOARD     = 1 << 22;
        const CREATE_THREADS     = 1 << 23;
        const MANAGE_THREADS     = 1 << 24;
    }
}

impl Permissions {
    /// What `@everyone` gets in a fresh guild.
    pub const DEFAULT_EVERYONE: Self = Self::VIEW_CHANNEL
        .union(Self::SEND_MESSAGES)
        .union(Self::READ_MESSAGE_HISTORY)
        .union(Self::ADD_REACTIONS)
        .union(Self::ATTACH_FILES)
        .union(Self::CONNECT)
        .union(Self::SPEAK)
        .union(Self::STREAM)
        .union(Self::SEND_VOICE_MESSAGES)
        .union(Self::USE_SOUNDBOARD)
        .union(Self::CREATE_INVITE)
        .union(Self::CREATE_THREADS);

    /// Apply a channel override: deny wins over an existing allow, then allow is added.
    pub fn apply_override(&mut self, allow: Self, deny: Self) {
        *self = (*self & !deny) | allow;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_wins_then_allow_applies() {
        let mut perms = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
        perms.apply_override(Permissions::empty(), Permissions::SEND_MESSAGES);
        assert!(perms.contains(Permissions::VIEW_CHANNEL));
        assert!(!perms.contains(Permissions::SEND_MESSAGES));

        perms.apply_override(Permissions::SEND_MESSAGES, Permissions::empty());
        assert!(perms.contains(Permissions::SEND_MESSAGES));
    }

    #[test]
    fn everyone_defaults_do_not_include_admin() {
        assert!(!Permissions::DEFAULT_EVERYONE.contains(Permissions::ADMINISTRATOR));
        assert!(Permissions::DEFAULT_EVERYONE.contains(Permissions::VIEW_CHANNEL));
    }
}
