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

while [ $# -gt 0 ]; do
	case "$1" in
	--repo) REPO="${2:?--repo needs owner/name}"; shift 2 ;;
	--version) VERSION="${2:?--version needs a tag}"; shift 2 ;;
	--prefix) PREFIX="${2:?--prefix needs a path}"; shift 2 ;;
	--no-start) START=0; shift ;;
	-h | --help)
		echo "usage: install.sh --repo owner/name [--version vX.Y.Z] [--no-start] [--prefix /opt/nova]"
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
install -d -m 755 /etc/nova

if [ ! -f /etc/nova/nova.env ]; then
	cat >/etc/nova/nova.env <<'EOF'
DATABASE_URL=postgres://nova:CHANGE_ME@127.0.0.1:5432/nova
NOVA_BIND_ADDR=127.0.0.1:8080
NOVA_PUBLIC_ORIGIN=https://nova.example.com
NOVA_TRUST_PROXY=true
NOVA_HASH_CONCURRENCY=1
NOVA_LISTEN_BACKLOG=4096
NOVA_UPDATE_REPO=
RUST_LOG=info
EOF
	chmod 600 /etc/nova/nova.env
	echo "==> wrote /etc/nova/nova.env — set DATABASE_URL and NOVA_PUBLIC_ORIGIN"
fi

if [ ! -f /etc/systemd/system/nova.service ]; then
	curl -fsSL "https://raw.githubusercontent.com/$REPO/$TAG/deploy/systemd/nova.service" \
		-o /etc/systemd/system/nova.service || true
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
