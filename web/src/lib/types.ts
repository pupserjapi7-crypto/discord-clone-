// IDs are decimal strings in JSON: Snowflakes exceed Number.MAX_SAFE_INTEGER.
export type Id = string;

export type UserPublic = {
  id: Id;
  handle: string;
  display_name: string;
  status: number;
  avatar_url: string | null;
};

export type GuildSummary = { id: Id; name: string };

export type Channel = {
  id: Id;
  guild_id: Id;
  name: string;
  kind: number;
  topic: string | null;
  parent_id: Id | null;
  position: number;
};

export type Member = {
  user_id: Id;
  handle: string;
  display_name: string;
  status: number;
  avatar_url: string | null;
};

export type GuildDetail = {
  guild: { id: Id; name: string; owner_id: Id };
  channels: Channel[];
  roles: unknown[];
  members: Member[];
  permissions: number;
};

export type Reaction = { emoji: string; count: number; me: boolean };

export type Message = {
  id: Id;
  channel_id: Id;
  author: UserPublic;
  content: string;
  created_at: string;
  edited_at: string | null;
  reply_to: Id | null;
  reactions: Reaction[];
  attachments: unknown[];
};
