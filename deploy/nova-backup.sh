#!/bin/sh
# Project Nova – nächtliches Backup: Datenbank-Dump und Uploads.
#
#   /usr/local/bin/nova-backup            (der Installer legt es samt Cronjob an)
#
# - Datenbank: `pg_dump -Fc` nach $DEST/db, ältere als $KEEP_DAYS Tage fliegen raus.
# - Uploads: content-addressed, eine Datei ändert sich nie. `cp` kopiert nur
#   neue Blobs und überschreibt nichts; gelöschte Blobs bleiben im Backup, damit
#   auch ältere Dumps vollständig wiederherstellbar sind.
#
# Wiederherstellen:
#   pg_restore --clean --if-exists -d "$DATABASE_URL" $DEST/db/nova-<zeit>.dump
#   cp -a --update=none $DEST/uploads/. $NOVA_UPLOAD_DIR/ && chown -R nova:nova $NOVA_UPLOAD_DIR
set -eu

ENV_FILE="${NOVA_ENV_FILE:-/etc/nova/nova.env}"
DEST="${NOVA_BACKUP_DIR:-/var/backups/nova}"
KEEP_DAYS="${NOVA_BACKUP_KEEP_DAYS:-7}"

if [ -f "$ENV_FILE" ]; then
	set -a
	# shellcheck disable=SC1090
	. "$ENV_FILE"
	set +a
fi
: "${DATABASE_URL:?DATABASE_URL fehlt (in $ENV_FILE)}"
UPLOADS="${NOVA_UPLOAD_DIR:-/var/lib/nova/uploads}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"

umask 077
mkdir -p "$DEST/db" "$DEST/uploads"

pg_dump -Fc --dbname="$DATABASE_URL" --file="$DEST/db/nova-$STAMP.dump.part"
mv "$DEST/db/nova-$STAMP.dump.part" "$DEST/db/nova-$STAMP.dump"
echo "db: $DEST/db/nova-$STAMP.dump"

if [ -d "$UPLOADS" ]; then
	# Nie überschreiben: coreutils ≥ 9.3 kennt --update=none, ältere nur -n.
	if cp -a --update=none --help >/dev/null 2>&1; then
		set -- -a --update=none
	else
		set -- -an
	fi
	# Nur die Shard-Verzeichnisse (zwei Hex-Zeichen), nicht tmp/.
	for shard in "$UPLOADS"/??; do
		if [ -d "$shard" ]; then
			cp "$@" "$shard" "$DEST/uploads/"
		fi
	done
	echo "uploads: $(find "$DEST/uploads" -type f | wc -l) Dateien"
fi

find "$DEST/db" -name 'nova-*.dump' -mtime +"$KEEP_DAYS" -delete
find "$DEST/db" -name 'nova-*.dump.part' -mtime +1 -delete
