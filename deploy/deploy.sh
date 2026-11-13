#!/usr/bin/env bash
# Atomic deploy: upload a prebuilt musl binary, swap the symlink, restart.
# Usage: deploy/deploy.sh <host> <path-to-nova-binary>
set -euo pipefail

host="${1:?usage: deploy.sh <user@host> <binary>}"
binary="${2:?usage: deploy.sh <user@host> <binary>}"
release="$(date -u +%Y%m%dT%H%M%SZ)"

ssh "$host" "mkdir -p /opt/nova/releases/$release"
scp "$binary" "$host:/opt/nova/releases/$release/nova"
ssh "$host" "chmod +x /opt/nova/releases/$release/nova"

# Migrate before switching traffic; migrations are additive and idempotent.
ssh "$host" "/opt/nova/releases/$release/nova migrate"

ssh "$host" "ln -sfn /opt/nova/releases/$release /opt/nova/current.new \
  && mv -Tf /opt/nova/current.new /opt/nova/current \
  && systemctl restart nova"

ssh "$host" "curl -fsS --retry 10 --retry-delay 1 http://127.0.0.1:8080/readyz > /dev/null"
echo "deployed $release"
