# Installation und Updates

Zwei Wege, immer die neueste GitHub-Release zu bekommen:

## 1. Installer (empfohlen, Erstinstallation und Update)

```bash
curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/install.sh \
  | sudo sh -s -- --repo <owner>/<repo>
```

Der Installer:

1. fragt die **neueste** Release über die GitHub-API ab (jeder Aufruf neu),
2. lädt `nova-linux-x86_64` und die zugehörige `.sha256`,
3. **verifiziert den Checksum vor** dem Austausch,
4. installiert nach `/opt/nova/releases/<tag>/nova` und schaltet `current` atomar um,
5. legt Benutzer `nova`, `/etc/nova/nova.env` und die systemd-Unit an (falls nicht vorhanden),
6. führt `nova migrate` aus und startet den Dienst neu.

Erneut ausführen ist der Update-Pfad. Optionen: `--version v0.2.0`, `--prefix`, `--no-start`.

Voraussetzungen: root, `curl`, `python3`, `sha256sum`, systemd, PostgreSQL erreichbar. `DATABASE_URL` und `NOVA_PUBLIC_ORIGIN` in `/etc/nova/nova.env` setzen.

## 2. In-App-Updater

Das Binary aktualisiert sich selbst aus derselben Release-Quelle:

```bash
nova update --check          # current=… latest=… (kein Download)
sudo nova update             # laden, prüfen, ersetzen, Dienst neu starten
sudo nova update --no-restart
```

Das Repo ist zur Compile-Zeit eingebettet (`NOVA_UPDATE_REPO`, gesetzt im Release-Workflow) oder wird zur Laufzeit über `--repo owner/name` / `NOVA_UPDATE_REPO` übergeben. Der Download wird **vor** dem atomaren `rename` gegen die Release-Checksumme geprüft; schlägt sie fehl, bleibt das laufende Binary unangetastet.

## 3. Release erzeugen

```bash
git tag v0.2.0
git push origin v0.2.0
```

`.github/workflows/release.yml` baut das Frontend, kompiliert `nova` und `nova-loadtest` für `x86_64-unknown-linux-musl`, erzeugt Checksummen und veröffentlicht sie als GitHub-Release. Installer und Updater finden sie danach automatisch.

## 4.manuell (Fallback)

```bash
curl -fsSL https://github.com/<owner>/<repo>/releases/latest/download/nova-linux-x86_64 -o nova
curl -fsSL https://github.com/<owner>/<repo>/releases/latest/download/nova-linux-x86_64.sha256 -o nova.sha256
sha256sum -c nova.sha256
sudo install -m 755 nova /opt/nova/releases/manual/nova
```

## Sicherheit

- Nur HTTPS, nur öffentliche Releases (kein Token im Binary).
- SHA-256 aus derselben Release; kein `curl | sh` des Binaries ohne Prüfung.
- `rename(2)` ist atomar; bei Fehlern bleibt die alte Version aktiv.
- Der Updater braucht Schreibrechte auf das Binary und `systemctl restart nova` (sudo-Regel in `docs/04-vps-bootstrap.md`).
