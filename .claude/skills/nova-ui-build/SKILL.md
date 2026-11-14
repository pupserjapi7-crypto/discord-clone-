---
name: nova-ui-build
description: Die exakte Build-Kette der Dioxus-Web-UI von Project Nova bis ins Server-Binary, inklusive WASM-Clippy. Laden nach jeder Änderung in crates/ui.
---

# Nova-UI bauen

Die UI wird zu WASM gebaut, nach `crates/ui/dist` kopiert und vom Server per
`rust-embed` ins Binary eingebettet. Ein UI-Fix ist erst live, wenn das
Server-Binary neu gebaut wurde.

## Ein Befehl

```bash
cargo xtask ui      # dx build --web --release --features web, dist neu befüllen, app.css kopieren
cargo xtask build   # zusätzlich: cargo build --release -p nova
```

## Manuell (entspricht `xtask ui`)

```bash
cd crates/ui && dx build --web --release --features web && cd ../..
rm -rf crates/ui/dist && mkdir -p crates/ui/dist
cp -a target/dx/nova-ui/release/web/public/. crates/ui/dist/
cp crates/ui/assets/app.css crates/ui/dist/assets/app.css
touch crates/nova/src/main.rs   # rust-embed-Cache invalidieren
cargo build -p nova
```

## Prüfen

```bash
cargo clippy -p nova-ui --target wasm32-unknown-unknown --features web -- -D warnings
```

- `dx` 0.7.0 (`cargo install dioxus-cli --version 0.7.0`). Auf dieser Maschine
  liegt `dx.exe` im Toolchain-Ordner, `DX_HOME=.dev/dx` (wasm-bindgen, wasm-opt).
- WASM-Größe nach dem Build notieren (`crates/ui/dist/assets/*.wasm`), Budget
  komprimiert < 2 MB.
- `crates/ui/dist` ist eingecheckt; die CI baut daraus das Server-Binary.
- Komponenten-Galerie: `cargo xtask gallery` baut mit Feature `gallery` und
  legt sie unter `/dev/gallery` ab. Nie mit `gallery` ausliefern.
