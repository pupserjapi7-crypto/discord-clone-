//! `cargo xtask` — die Build-Kette von Project Nova in einem Rust-Programm.
//!
//! Ersetzt die Shell-Schritte aus der README, damit Windows, Linux und CI
//! dasselbe tun:
//!
//! - `ui`      Dioxus-Web-Build → `crates/ui/dist` → `app.css` kopieren
//! - `gallery` wie `ui`, aber mit Feature `gallery` (nur lokal, nie ausliefern)
//! - `build`   `ui` + `cargo build --release -p nova`
//! - `check`   fmt, clippy (Workspace + WASM), Tests
//! - `all`     `ui` + `check` + Server-Build

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_default();
    let root = workspace_root()?;
    match task.as_str() {
        "ui" => ui(&root, false),
        "gallery" => ui(&root, true),
        "build" => {
            ui(&root, false)?;
            server(&root)
        }
        "check" => check(&root),
        "all" => {
            ui(&root, false)?;
            check(&root)?;
            server(&root)
        }
        _ => {
            eprintln!(
                "Aufruf: cargo xtask <ui|gallery|build|check|all>\n\n\
                 ui       Web-UI bauen und nach crates/ui/dist legen\n\
                 gallery  Web-UI mit Komponenten-Galerie (/dev/gallery), nur lokal\n\
                 build    ui + Server-Binary (release)\n\
                 check    fmt, clippy (Workspace + WASM), Tests\n\
                 all      ui + check + Server-Binary"
            );
            if task.is_empty() {
                Ok(())
            } else {
                bail!("unbekannte Aufgabe: {task}")
            }
        }
    }
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .context("xtask liegt nicht im Workspace")
}

fn run(dir: &Path, program: &str, args: &[&str]) -> Result<()> {
    println!("» {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .with_context(|| format!("{program} konnte nicht gestartet werden"))?;
    if !status.success() {
        bail!("{program} {} ist fehlgeschlagen ({status})", args.join(" "));
    }
    Ok(())
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// Baut die Web-UI und legt sie dort ab, wo `rust-embed` sie erwartet.
fn ui(root: &Path, gallery: bool) -> Result<()> {
    let features = if gallery { "web,gallery" } else { "web" };
    let public = root.join("target/dx/nova-ui/release/web/public");
    // dx räumt sein Ausgabeverzeichnis nicht auf; alte WASM-Dateien würden
    // sonst mit eingebettet.
    if public.exists() {
        fs::remove_dir_all(&public).context("altes dx-Ausgabeverzeichnis entfernen")?;
    }
    run(
        &root.join("crates/ui"),
        "dx",
        &["build", "--web", "--release", "--features", features],
    )?;

    let dist = root.join("crates/ui/dist");
    if dist.exists() {
        fs::remove_dir_all(&dist).context("crates/ui/dist leeren")?;
    }
    copy_dir(&public, &dist)?;
    fs::create_dir_all(dist.join("assets"))?;
    fs::copy(
        root.join("crates/ui/assets/app.css"),
        dist.join("assets/app.css"),
    )
    .context("app.css kopieren")?;

    // rust-embed merkt geänderte Dateien nicht immer: Server-Crate anstoßen.
    let main = root.join("crates/nova/src/main.rs");
    let text = fs::read(&main)?;
    fs::write(&main, text)?;

    for entry in fs::read_dir(dist.join("assets"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "wasm") {
            let size = fs::metadata(&path)?.len();
            println!(
                "WASM {}: {} KB",
                path.file_name().unwrap_or_default().to_string_lossy(),
                size / 1024
            );
        }
    }
    if gallery {
        println!(
            "Galerie-Build liegt in crates/ui/dist – vor dem Commit `cargo xtask ui` ausführen!"
        );
    }
    Ok(())
}

fn server(root: &Path) -> Result<()> {
    run(root, &cargo(), &["build", "--release", "-p", "nova"])
}

fn check(root: &Path) -> Result<()> {
    let cargo = cargo();
    run(root, &cargo, &["fmt", "--all", "--check"])?;
    run(
        root,
        &cargo,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(
        root,
        &cargo,
        &[
            "clippy",
            "-p",
            "nova-ui",
            "--target",
            "wasm32-unknown-unknown",
            "--features",
            "web",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(root, &cargo, &["test", "--workspace"])
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("{from:?} lesen"))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
