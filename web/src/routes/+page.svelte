<script lang="ts">
  import { onMount } from 'svelte';
  import { api, auth, getToken, setToken } from '$lib/api';
  import { Gateway } from '$lib/gateway';
  import type { Channel, GuildDetail, GuildSummary, Message, UserPublic } from '$lib/types';

  let user = $state<UserPublic | null>(null);
  let guilds = $state<GuildSummary[]>([]);
  let detail = $state<GuildDetail | null>(null);
  let channel = $state<Channel | null>(null);
  let messages = $state<Message[]>([]);
  let status = $state<'connecting' | 'online' | 'offline'>('offline');
  let error = $state('');
  let draft = $state('');
  let handle = $state('load0000');
  let password = $state('loadtest-password');
  let registerMode = $state(false);

  let gateway: Gateway | null = null;
  let nonce = 0;
  let pending = new Map<number, string>();

  onMount(async () => {
    try {
      user = await auth.refresh();
      await boot();
    } catch {
      /* show login */
    }
  });

  async function boot() {
    guilds = await api<GuildSummary[]>('/api/v1/guilds');
    if (guilds[0]) await openGuild(guilds[0].id);
  }

  async function openGuild(id: string) {
    detail = await api<GuildDetail>(`/api/v1/guilds/${id}`);
    const text = detail.channels.find((c) => c.kind === 0);
    if (text) await openChannel(text.id);
  }

  async function openChannel(id: string) {
    channel = detail?.channels.find((c) => c.id === id) ?? null;
    messages = await api<Message[]>(`/api/v1/channels/${id}/messages?limit=50`);
    if (gateway) gateway.setChannel(id);
  }

  function connect() {
    if (!channel || !getToken()) return;
    gateway?.close();
    gateway = new Gateway({
      onMessage: (message) => {
        messages = [...messages.filter((m) => m.id !== message.id), message];
        if (detail) detail = { ...detail };
      },
      onAck: (ackNonce, id) => {
        const localId = pending.get(ackNonce);
        if (localId === undefined) return;
        pending.delete(ackNonce);
        messages = messages.map((m) => (m.id === localId ? { ...m, id: String(id) } : m));
      },
      onStatus: (next) => (status = next),
      onError: (message) => (error = message),
      onResync: async () => {
        // The replay window was exceeded: refetch instead of hiding a gap.
        if (channel) messages = await api<Message[]>(`/api/v1/channels/${channel.id}/messages?limit=50`);
      }
    });
    gateway.connect(getToken()!, channel.id);
  }

  async function submitLogin(event: SubmitEvent) {
    event.preventDefault();
    error = '';
    try {
      user = registerMode
        ? await auth.register(handle, password)
        : await auth.login(handle, password);
      registerMode = false;
      await boot();
      connect();
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  }

  async function logout() {
    gateway?.close();
    gateway = null;
    await auth.logout().catch(() => setToken(null));
    user = null;
    messages = [];
    guilds = [];
    detail = null;
    channel = null;
  }

  function send(event: SubmitEvent) {
    event.preventDefault();
    const content = draft.trim();
    if (!content || !channel || !gateway) return;
    const localId = `local-${++nonce}`;
    pending.set(nonce, localId);
    messages = [
      ...messages,
      {
        id: localId,
        channel_id: channel.id,
        author: user!,
        content,
        created_at: new Date().toISOString(),
        edited_at: null,
        reply_to: null,
        reactions: [],
        attachments: []
      }
    ];
    gateway.sendMessage(nonce, channel.id, content);
    draft = '';
  }

  function time(iso: string) {
    return new Date(iso).toLocaleTimeString('de-DE', { hour: '2-digit', minute: '2-digit' });
  }
</script>

<svelte:head><title>Nova</title></svelte:head>

{#if !user}
  <main class="flex min-h-screen items-center justify-center p-6">
    <form
      class="w-full max-w-sm rounded-panel border border-line bg-surface p-6"
      onsubmit={submitLogin}
    >
      <h1 class="text-2xl font-semibold tracking-tight">Nova</h1>
      <p class="mt-1 text-ink2">{registerMode ? 'Neues Konto anlegen.' : 'Melde dich an, um weiterzumachen.'}</p>
      <label class="mt-6 block text-xs uppercase tracking-wide text-ink3" for="handle">Handle</label>
      <input id="handle" bind:value={handle} class="mt-1 w-full rounded-card border border-line bg-surface2 px-3 py-2 outline-none" autocomplete="username" />
      <label class="mt-4 block text-xs uppercase tracking-wide text-ink3" for="password">Passwort</label>
      <input id="password" type="password" bind:value={password} class="mt-1 w-full rounded-card border border-line bg-surface2 px-3 py-2 outline-none" autocomplete="current-password" />
      {#if error}<p class="mt-3 text-sm" style="color: var(--nx-danger)">{error}</p>{/if}
      <button class="mt-6 w-full rounded-card bg-accent px-3 py-2 font-medium text-white transition-opacity duration-150 hover:opacity-90" type="submit">
        {registerMode ? 'Registrieren' : 'Anmelden'}
      </button>
      <button
        type="button"
        class="mt-3 w-full text-center text-xs text-ink3 hover:text-ink"
        onclick={() => { registerMode = !registerMode; error = ''; }}
      >
        {registerMode ? 'Ich habe schon ein Konto' : 'Neues Konto erstellen'}
      </button>
      <p class="mt-4 text-xs text-ink3">
        Erster Start? <code>nova seed</code> legt Testkonten an. Registrierung: POST /api/v1/auth/register
      </p>
    </form>
  </main>
{:else}
  <div class="flex h-screen overflow-hidden">
    <nav class="flex w-[72px] flex-col items-center gap-2 border-r border-line bg-[var(--nx-bg-sunken)] py-3" aria-label="Server">
      {#each guilds as guild (guild.id)}
        <button
          class="grid h-11 w-11 place-items-center rounded-card bg-surface2 text-sm font-semibold transition-transform duration-150 hover:scale-105"
          onclick={() => openGuild(guild.id)}
          title={guild.name}
        >{guild.name.slice(0, 2).toUpperCase()}</button>
      {/each}
    </nav>

    <aside class="flex w-60 flex-col border-r border-line bg-surface">
      <header class="border-b border-line px-4 py-3">
        <p class="text-sm font-semibold">{detail?.guild.name ?? 'Nova'}</p>
        <p class="text-xs text-ink3">{status === 'online' ? 'verbunden' : status}</p>
      </header>
      <ul class="flex-1 overflow-y-auto p-2">
        {#each detail?.channels ?? [] as item (item.id)}
          <li>
            <button
              class="flex w-full items-center gap-2 rounded-card px-3 py-2 text-left transition-colors duration-150 {item.id === channel?.id ? 'bg-[var(--nx-accent-soft)] text-ink' : 'text-ink2 hover:bg-surface2'}"
              onclick={() => openChannel(item.id)}
            >
              <span class="text-ink3">{item.kind === 0 ? '#' : '🔊'}</span>{item.name}
            </button>
          </li>
        {/each}
      </ul>
      <footer class="flex items-center justify-between border-t border-line px-3 py-2">
        <span class="truncate text-sm">{user.display_name}</span>
        <button class="text-xs text-ink3 hover:text-ink" onclick={logout}>Logout</button>
      </footer>
    </aside>

    <main class="flex min-w-0 flex-1 flex-col">
      <header class="flex items-center justify-between border-b border-line px-4 py-3">
        <div>
          <h2 class="text-sm font-semibold">{channel ? `# ${channel.name}` : 'Kanal wählen'}</h2>
          <p class="text-xs text-ink3">{channel?.topic ?? 'Kein Thema gesetzt'}</p>
        </div>
        {#if error}<span class="text-xs" style="color: var(--nx-danger)">{error}</span>{/if}
      </header>

      <ol class="flex-1 space-y-3 overflow-y-auto px-4 py-4" role="log" aria-live="polite">
        {#each messages as message (message.id)}
          <li class="rounded-card px-2 py-1">
            <div class="flex items-baseline gap-2">
              <span class="text-sm font-medium">{message.author.display_name}</span>
              <span class="text-[11px] text-ink3">{time(message.created_at)}</span>
            </div>
            <p class="whitespace-pre-wrap break-words text-[15px] leading-relaxed">{message.content}</p>
            {#if message.reactions.length}
              <div class="mt-1 flex gap-1">
                {#each message.reactions as reaction (reaction.emoji)}
                  <span class="rounded-full border border-line px-2 py-0.5 text-xs">{reaction.emoji} {reaction.count}</span>
                {/each}
              </div>
            {/if}
          </li>
        {/each}
        {#if messages.length === 0}
          <li class="py-10 text-center text-sm text-ink3">Noch keine Nachrichten — schreib die erste.</li>
        {/if}
      </ol>

      <form class="border-t border-line p-3" onsubmit={send}>
        <input
          bind:value={draft}
          class="w-full rounded-card border border-line bg-surface2 px-3 py-3 outline-none"
          placeholder={channel ? `Nachricht an #${channel.name}` : 'Kanal wählen'}
          disabled={!channel}
          aria-label="Nachricht"
        />
      </form>
    </main>
  </div>
{/if}
