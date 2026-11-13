#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let target = std::env::var("NOVA_DESKTOP_URL")
        .unwrap_or_else(|_| "http://85.215.175.202".to_owned());

    tauri::Builder::default()
        .setup(move |app| {
            let url: tauri::Url = target.parse()?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
                .title("Nova")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Nova");
}
