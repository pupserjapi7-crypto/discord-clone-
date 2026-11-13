# VPS-Bootstrap: eigener SSH-Benutzer, Key, Härtung

Ziel: kein Root-Login, kein Passwort-Login, nur `nova` mit Key und sudo für Deploy/Restart. Alle Befehle als root in einer **bestehenden** Session ausführen und diese erst schließen, wenn der neue Zugang getestet ist.

## 1. Benutzer und Key

Auf dem **lokalen Rechner** (falls noch kein Key existiert):

```bash
ssh-keygen -t ed25519 -a 100 -C "nova-deploy" -f ~/.ssh/nova_deploy
```

Auf dem Server:

```bash
adduser --disabled-password --gecos "" nova
install -d -m 700 -o nova -g nova /home/nova/.ssh
# Inhalt von nova_deploy.pub hier einfügen:
echo "ssh-ed25519 AAAA... nova-deploy" > /home/nova/.ssh/authorized_keys
chown nova:nova /home/nova/.ssh/authorized_keys
chmod 600 /home/nova/.ssh/authorized_keys
```

Passwortlosen sudo nur für Deploy und Dienststeuerung:

```bash
cat >/etc/sudoers.d/nova <<'EOF'
nova ALL=(root) NOPASSWD: /usr/bin/systemctl restart nova, /usr/bin/systemctl status nova, /usr/bin/systemctl reload caddy, /opt/nova/deploy.sh
EOF
chmod 440 /etc/sudoers.d/nova
```

## 2. sshd härten

```bash
cp /etc/ssh/sshd_config /etc/ssh/sshd_config.bak
cat >/etc/ssh/sshd_config.d/99-nova.conf <<'EOF'
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers nova
MaxAuthTries 3
LoginGraceTime 20
ClientAliveInterval 300
ClientAliveCountMax 2
EOF
sshd -t && systemctl reload ssh
```

**Vor dem Schließen der Root-Session** in einem zweiten Terminal testen:

```bash
ssh -i ~/.ssh/nova_deploy nova@<server> 'id && sudo systemctl is-active ssh'
```

Erst wenn das funktioniert, die Root-Session beenden. Bei Aussperrung: Provider-Konsole (IONOS) nutzen.

## 3. Firewall und Fail2ban

```bash
apt update && apt install -y ufw fail2ban
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp comment 'ssh'
ufw allow 80,443/tcp comment 'caddy'
ufw --force enable

cat >/etc/fail2ban/jail.d/nova.local <<'EOF'
[sshd]
enabled = true
maxretry = 4
findtime = 10m
bantime = 1h
EOF
systemctl enable --now fail2ban
```

## 4. Laufzeit

```bash
apt install -y postgresql-17 caddy
sudo -u postgres createuser --createdb nova
sudo -u postgres psql -c "alter user nova password '<starkes-passwort>';"
sudo -u postgres createdb -O nova nova
```

`/etc/nova/nova.env` (root:root, `0600`):

```ini
DATABASE_URL=postgres://nova:<passwort>@127.0.0.1:5432/nova
NOVA_BIND_ADDR=127.0.0.1:8080
NOVA_PUBLIC_ORIGIN=https://nova.example.com
NOVA_TRUST_PROXY=true
NOVA_HASH_CONCURRENCY=1
NOVA_LISTEN_BACKLOG=4096
RUST_LOG=info
```

Binary und Units:

```bash
install -d -o nova -g nova /opt/nova/releases /var/lib/nova
# von CI heruntergeladenes Binary:
scp nova-x86_64-linux-musl nova@<server>:/opt/nova/releases/<datum>/nova
install -m 644 deploy/systemd/nova.service /etc/systemd/system/nova.service
install -m 644 deploy/caddy/Caddyfile /etc/caddy/Caddyfile
systemctl daemon-reload
systemctl enable --now nova caddy
```

Prüfen:

```bash
curl -fsS http://127.0.0.1:8080/healthz
curl -fsS http://127.0.0.1:8080/readyz
journalctl -u nova -n 50 --no-pager
```

## 5. Was der Server danach können muss

- `ssh nova@host` **nur** mit Key; `ssh -o PasswordAuthentication=yes` scheitert.
- `ufw status` zeigt nur 22/80/443.
- `systemctl status nova` aktiv, `MemoryMax=768M`, `LimitNOFILE=65536`.
- `nova doctor` verbindet zur DB und meldet Nutzer/Guilds.

Sobald der Zugang steht, kann der erste Deploy (`deploy/deploy.sh`) und der isolierte Lasttest auf dem VPS laufen.
