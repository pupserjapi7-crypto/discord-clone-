# Production deployment (Plesk VPS)

Live endpoint: **http://85.215.175.202** (HTTP only, no domain/TLS yet).

## Topology

| Part | Value |
| --- | --- |
| Host | Debian 13, 2 vCPU, 1.841 MB RAM, Plesk Obsidian 18.0.82.2 |
| Nova binary | `/opt/nova/current/nova` (currently 0.1.4) |
| Service | `nova.service`, user `nova`, `MemoryMax=900M` |
| App listener | `0.0.0.0:8080` (blocked externally; loopback only in practice) |
| Public entry | Plesk Apache IP vhost `85.215.175.202:80` reverse-proxies to `127.0.0.1:8080` |
| WebSocket | `ProxyPass /gateway ws://127.0.0.1:8080/gateway` (mod_proxy_wstunnel) |
| Database | PostgreSQL 17, database/user `nova`, password in `/etc/nova/nova.env` |
| Uploads | `/var/lib/nova/uploads` |
| Backups | `/usr/local/bin/nova-backup`, cron 03:30, `/var/backups/nova`, 7 days |
| Firewall | ufw active; every port listening at enable time allowed, DB ports removed |
| Brute force | fail2ban sshd jail (5 failures / 10 min → 1 h ban) |
| SSH | key-only; `PermitRootLogin prohibit-password`, `PasswordAuthentication no` |

## Environment (`/etc/nova/nova.env`, mode 600)

```ini
NOVA_BIND_ADDR=0.0.0.0:8080
NOVA_PUBLIC_ORIGIN=http://85.215.175.202
NOVA_TRUST_PROXY=true
NOVA_HASH_CONCURRENCY=1
NOVA_LISTEN_BACKLOG=4096
NOVA_UPLOAD_DIR=/var/lib/nova/uploads
NOVA_AUTH_BURST=60
NOVA_AUTH_RATE=10
NOVA_MSG_RATE=10
NOVA_MSG_BURST=30
NOVA_UPDATE_REPO=pupserjapi7-crypto/discord-clone-
```

`NOVA_TRUST_PROXY=true` is required because Apache forwards `X-Forwarded-For`;
rate limiting keys on the forwarded client IP.

## Deploy / update

```bash
# as root
curl -fsSL https://github.com/pupserjapi7-crypto/discord-clone-/releases/latest/download/nova-linux-x86_64 -o /tmp/nova
# or, in place:
su -s /bin/sh nova -c 'NOVA_UPDATE_REPO=pupserjapi7-crypto/discord-clone- /opt/nova/current/nova update --no-restart'
systemctl restart nova
```

`nova update` verifies the SHA-256 asset before swapping the release symlink.
The service restart is done by root because the updater does not elevate itself.

## Apache overrides (Plesk may regenerate `server.conf`)

Plesk owns `/etc/apache2/plesk.conf.d/server.conf`. The install appended these
directives to the `85.215.175.202:80` vhost and kept
`server.conf.nova.bak`. After a Plesk reconfiguration, check the vhost and
re-add them if `/` stops returning the Nova UI:

```apache
ProxyPreserveHost On
ProxyPass /gateway ws://127.0.0.1:8080/gateway
ProxyPassReverse /gateway ws://127.0.0.1:8080/gateway
ProxyPass / http://127.0.0.1:8080/
ProxyPassReverse / http://127.0.0.1:8080/
```

Worker limit (`/etc/apache2/mods-available/mpm_event.conf`, restart Apache):
`ServerLimit 12`, `ThreadLimit 256`, `ThreadsPerChild 128`,
`MaxRequestWorkers 1536`. Default 150 workers cannot hold 1.000 WebSockets.

## Measured on this host (2026-11-13)

| Test | Result |
| --- | --- |
| 1.000 receivers via public URL | 1.000 connected, 0 failed, 0 lagged, 0 resyncs, RSS ≈ 165 MB |
| 20 senders + 1.000 receivers, 60 s, 1 msg/s each | 1.175 sent/acked, **1.174.586 events**, 0 gaps, 0 lagged, 0 resyncs, 0 reconnects |
| Server dispatch latency | p50 0.22 ms, p95 2.28 ms, p99 3.30 ms |
| Message pipeline latency | p50 3.2 ms, p95 23.8 ms, p99 52.3 ms |
| ACK round-trip from home uplink | p50 209 ms, p95 676 ms, p99 1.14 s (client/network bound; 55 ms without fan-out) |
| RSS after load | 228 MB peak, ~60 MB idle |
| CPU | ~42 s CPU over 6 min uptime including both load tests |

Raw protocol tests are independent of the UI. Die frühere Playwright-Suite
(`web/e2e/smoke.spec.ts`) wurde mit dem SvelteKit-Frontend entfernt. Der
Rust-UI-Smoke läuft lokal gegen `http://127.0.0.1:8080` (Registrierung, Guild,
Kanal, Nachricht) und ist in `docs/benchmarks/m1-gateway.md` als historische
Messung dokumentiert.

## Known limitations

- No TLS and no domain yet: Plesk owns 80/443 and there is no DNS name. Add a
  Plesk domain + Let's Encrypt certificate, then move the proxy to that vhost
  and set `NOVA_PUBLIC_ORIGIN=https://…`.
- Presence is not fanned out on connect/disconnect (it was O(n²)); online
  status comes from the guild member list, profile updates still broadcast.
- Single node only. The resume ring and rate limits are in-process, so a second
  instance would need sticky sessions or a shared gateway.
- Apache is the connection bottleneck above ~1.500 concurrent sockets on this
  box; 1.000 is the validated target.
