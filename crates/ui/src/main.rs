//! Nova Web-UI (Dioxus 0.7, WASM). Ein Store aus Signalen, Views lesen nur,
//! was sie brauchen. Struktur: siehe Skill `nova-ui-component`.

mod actions;
mod api;
mod components;
#[cfg(feature = "gallery")]
mod gallery;
mod gateway;
mod protocol;
mod settings;
mod store;
mod types;
mod views;
mod voice;

use dioxus::prelude::*;

use components::Icon;
use store::Store;

fn main() {
    dioxus::launch(Root);
}

#[component]
fn Root() -> Element {
    #[cfg(feature = "gallery")]
    if gallery::active() {
        return rsx! {
            link { rel: "stylesheet", href: "/assets/app.css" }
            gallery::Gallery {}
        };
    }
    rsx! {
        link { rel: "stylesheet", href: "/assets/app.css" }
        App {}
    }
}

#[component]
fn App() -> Element {
    let store = use_context_provider(Store::create);
    use_hook(move || spawn(actions::boot(store)));
    if *store.booting.read() {
        return rsx! {
            div { class: "nv-boot", role: "status",
                span { class: "nv-auth__logo", Icon { name: "logo", size: 24 } }
                "Sitzung wird geprüft …"
            }
        };
    }
    if store.user.read().is_none() {
        return rsx! { views::auth::AuthScreen {} };
    }
    rsx! { views::shell::Shell {} }
}
