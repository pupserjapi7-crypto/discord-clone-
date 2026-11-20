#!/bin/sh
# Project Nova server installer.
#
#   curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/install.sh | sh -s -- --repo <owner>/<repo>
#
# Always installs the newest GitHub release (or --version). Verifies the
# SHA-256 checksum before touching the running binary and swaps the symlink
# atomically. Safe to re-run: that is the update path.
set -eu

REPO="${NOVA_REPO:-}"
VERSION="${NOVA_VERSION:-latest}"
PREFIX="${NOVA_PREFIX:-/opt/nova}"
START=1
BACKUP=1

while [ $# -gt 0 ]; do
	case "$1" in
	--repo) REPO="${2:?--repo needs owner/name}"; shift 2 ;;
	--version) VERSION="${2:?--version needs a tag}"; shift 2 ;;
	--prefix) PREFIX="${2:?--prefix needs a path}"; shift 2 ;;
	--no-start) START=0; shift ;;
	--no-backup) BACKUP=0; shift ;;
	-h | --help)
		echo "usage: install.sh --repo owner/name [--version vX.Y.Z] [--no-start] [--no-backup] [--prefix /opt/nova]"
		exit 0
		;;
	*)
		echo "unknown option: $1" >&2
		exit 2
		;;
	esac
done

if [ -z "$REPO" ]; then
	echo "error: pass --repo owner/name (or set NOVA_REPO)" >&2
	exit 2
fi
if [ "$(id -u)" -ne 0 ]; then
	echo "error: run as root" >&2
	exit 1
fi
for tool in curl python3 sha256sum install systemctl; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "error: missing $tool" >&2
		exit 1
	}
done

if [ "$VERSION" = "latest" ]; then
	API="https://api.github.com/repos/$REPO/releases/latest"
else
	API="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
fi

echo "==> resolving $REPO ($VERSION)"
JSON="$(curl -fsSL -H 'Accept: application/vnd.github+json' "$API")"

asset_url() {
	printf '%s' "$JSON" | python3 -c '
import json, sys
release = json.load(sys.stdin)
name = sys.argv[1]
for asset in release["assets"]:
    if asset["name"] == name:
        print(asset["browser_download_url"])
        break
else:
    raise SystemExit("asset not found: " + name)
' "$1"
}

TAG="$(printf '%s' "$JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')"
BINARY_URL="$(asset_url nova-linux-x86_64)"
CHECKSUM_URL="$(asset_url nova-linux-x86_64.sha256)"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

echo "==> downloading $TAG"
curl -fsSL "$BINARY_URL" -o "$TMP/nova"
curl -fsSL "$CHECKSUM_URL" -o "$TMP/nova.sha256"

EXPECTED="$(awk '{print $1}' "$TMP/nova.sha256")"
ACTUAL="$(sha256sum "$TMP/nova" | awk '{print $1}')"
if [ "$EXPECTED" != "$ACTUAL" ]; then
	echo "error: checksum mismatch (expected $EXPECTED, got $ACTUAL)" >&2
	exit 1
fi
echo "==> checksum ok ($ACTUAL)"

RELEASE_DIR="$PREFIX/releases/$TAG"
install -d -m 755 "$PREFIX/releases" "$RELEASE_DIR"
install -m 755 "$TMP/nova" "$RELEASE_DIR/nova"
ln -sfn "$RELEASE_DIR" "$PREFIX/current.new"
mv -Tf "$PREFIX/current.new" "$PREFIX/current"
echo "==> installed $PREFIX/current -> $RELEASE_DIR"

if ! id nova >/dev/null 2>&1; then
	useradd --system --home /var/lib/nova --shell /usr/sbin/nologin nova
fi
install -d -o nova -g nova /var/lib/nova
# Uploads (content-addressed Blobs). Die systemd-Unit erlaubt Schreiben nur
# unter /var/lib/nova; das Verzeichnis gehört dem Dienstkonto.
install -d -o nova -g nova -m 750 /var/lib/nova/uploads
install -d -m 755 /etc/nova

if [ ! -f /etc/nova/nova.env ]; then
	cat >/etc/nova/nova.env <<'EOF'
DATABASE_URL=postgres://nova:CHANGE_ME@127.0.0.1:5432/nova
NOVA_BIND_ADDR=127.0.0.1:8080
NOVA_PUBLIC_ORIGIN=https://nova.example.com
NOVA_TRUST_PROXY=true
NOVA_HASH_CONCURRENCY=1
NOVA_LISTEN_BACKLOG=4096
NOVA_UPLOAD_DIR=/var/lib/nova/uploads
# Optional: NOVA_MAX_UPLOAD_BYTES=26214400, NOVA_GUILD_UPLOAD_QUOTA_BYTES=2147483648
# Sprache: Personen pro Sprachkanal (Standard 15), UDP 40000–40100 freigeben.
# Optional: NOVA_VOICE_MAX_USERS=15
NOVA_UPDATE_REPO=
RUST_LOG=info
EOF
	# Öffentliche IP für den SFU (ICE): Adresse der Standardroute. Hinter NAT
	# ist das eine private Adresse – dann von Hand eintragen.
	PUBLIC_IP=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}' | head -n 1)
	printf '# Öffentliche IP des Servers für Sprache (Pflicht, sonst scheitert ICE still)\nNOVA_SFU_PUBLIC_IP=%s\n' "$PUBLIC_IP" >>/etc/nova/nova.env
	chmod 600 /etc/nova/nova.env
	echo "==> wrote /etc/nova/nova.env — set DATABASE_URL and NOVA_PUBLIC_ORIGIN, check NOVA_SFU_PUBLIC_IP=${PUBLIC_IP:-?}"
fi

# Ohne öffentliche IP kündigt der SFU 127.0.0.1 an: Sprache verbindet dann nie.
if ! grep -q '^NOVA_SFU_PUBLIC_IP=[0-9a-fA-F]' /etc/nova/nova.env; then
	echo "==> WARNUNG: NOVA_SFU_PUBLIC_IP fehlt in /etc/nova/nova.env – Sprachkanäle funktionieren erst damit"
fi

# Ältere Installationen: ohne NOVA_UPLOAD_DIR zeigt der Standard auf
# ./data/uploads unter /opt (schreibgeschützt) und der Dienst startet nicht.
if ! grep -q '^NOVA_UPLOAD_DIR=' /etc/nova/nova.env; then
	printf '\n# Uploads (ab v0.8)\nNOVA_UPLOAD_DIR=/var/lib/nova/uploads\n' >>/etc/nova/nova.env
	echo "==> added NOVA_UPLOAD_DIR=/var/lib/nova/uploads to /etc/nova/nova.env"
fi

if [ ! -f /etc/systemd/system/nova.service ]; then
	curl -fsSL "https://raw.githubusercontent.com/$REPO/$TAG/deploy/systemd/nova.service" \
		-o /etc/systemd/system/nova.service || true
fi

# Nächtliches Backup (Datenbank + Uploads). Vorhandene Skripte und Cronjobs
# bleiben unangetastet; --no-backup überspringt den Schritt.
if [ "$BACKUP" -eq 1 ]; then
	if [ ! -f /usr/local/bin/nova-backup ] &&
		curl -fsSL "https://raw.githubusercontent.com/$REPO/$TAG/deploy/nova-backup.sh" -o "$TMP/nova-backup"; then
		install -m 755 "$TMP/nova-backup" /usr/local/bin/nova-backup
		echo "==> installed /usr/local/bin/nova-backup"
	fi
	if [ -f /usr/local/bin/nova-backup ] && [ -d /etc/cron.d ] && [ ! -f /etc/cron.d/nova-backup ] &&
		! crontab -l 2>/dev/null | grep -q nova-backup; then
		echo '30 3 * * * root /usr/local/bin/nova-backup >/var/log/nova-backup.log 2>&1' >/etc/cron.d/nova-backup
		chmod 644 /etc/cron.d/nova-backup
		echo "==> nightly backup at 03:30 (/etc/cron.d/nova-backup)"
	fi
fi

systemctl daemon-reload
if [ -f /etc/nova/nova.env ]; then
	set -a
	# shellcheck disable=SC1091
	. /etc/nova/nova.env
	set +a
	"$PREFIX/current/nova" migrate || echo "warning: migration failed, check DATABASE_URL"
fi

if [ "$START" -eq 1 ]; then
	systemctl enable nova >/dev/null 2>&1 || true
	systemctl restart nova
	sleep 1
	curl -fsS http://127.0.0.1:8080/healthz && echo " — nova is healthy"
else
	echo "==> skipped start (--no-start)"
fi

echo "==> done: $TAG"
