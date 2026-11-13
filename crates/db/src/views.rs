use nova_common::protocol::{AttachmentView, MessageView, ReactionSummary, UserPublic};

use crate::models::{Attachment, MessageRow, ReactionRow, User};

pub fn attachment_url(base_url: &str, id: i64) -> String {
    format!("{}/api/v1/attachments/{id}", base_url.trim_end_matches('/'))
}

pub fn attachment_view(base_url: &str, attachment: &Attachment) -> AttachmentView {
    AttachmentView {
        id: attachment.id,
        filename: attachment.filename.clone(),
        content_type: attachment.content_type.clone(),
        size: attachment.size,
        width: attachment.width,
        height: attachment.height,
        url: attachment_url(base_url, attachment.id),
    }
}

pub fn user_public(base_url: &str, user: &User) -> UserPublic {
    UserPublic {
        id: user.id,
        handle: user.handle.clone(),
        display_name: user.display_name.clone(),
        avatar_url: user.avatar_id.map(|id| attachment_url(base_url, id)),
        status: user.status,
    }
}

pub fn message_view(
    base_url: &str,
    row: MessageRow,
    reactions: Vec<ReactionRow>,
    attachments: Vec<Attachment>,
) -> MessageView {
    MessageView {
        id: row.id,
        channel_id: row.channel_id,
        author: UserPublic {
            id: row.author_id,
            handle: row.author_handle,
            display_name: row.author_display_name,
            avatar_url: row.author_avatar_id.map(|id| attachment_url(base_url, id)),
            status: row.author_status,
        },
        content: row.content,
        created_at: row.created_at,
        edited_at: row.edited_at,
        reply_to: row.reply_to,
        reactions: reactions
            .into_iter()
            .map(|r| ReactionSummary {
                emoji: r.emoji,
                count: r.count,
                me: r.me,
            })
            .collect(),
        attachments: attachments
            .iter()
            .map(|a| attachment_view(base_url, a))
            .collect(),
    }
}
