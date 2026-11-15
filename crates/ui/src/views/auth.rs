use dioxus::prelude::*;

use crate::{
    actions, api,
    components::{Avatar, Icon},
    store::use_store,
};

#[component]
pub fn AuthScreen() -> Element {
    let store = use_store();
    let mut handle = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut register = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);

    let submit = move |event: FormEvent| {
        event.prevent_default();
        let name = handle.read().trim().to_lowercase();
        let secret = password.read().clone();
        let registering = register();
        if name.len() < 3 {
            error.set(Some(
                "Der Benutzername braucht mindestens 3 Zeichen.".into(),
            ));
            return;
        }
        if registering && secret.chars().count() < 10 {
            error.set(Some("Das Passwort braucht mindestens 10 Zeichen.".into()));
            return;
        }
        busy.set(true);
        error.set(None);
        spawn(async move {
            match api::login(&name, &secret, registering).await {
                Ok(user) => actions::finish_login(store, user).await,
                Err(err) => {
                    let text = if !registering && err.contains("abgelaufen") {
                        "Benutzername oder Passwort stimmt nicht. Prüfe beides und versuch es noch mal."
                            .to_string()
                    } else {
                        err
                    };
                    error.set(Some(text));
                }
            }
            busy.set(false);
        });
    };

    let title = if register() {
        "Konto erstellen"
    } else {
        "Willkommen zurück"
    };
    let invalid = if error.read().is_some() {
        "true"
    } else {
        "false"
    };
    rsx! {
        main { class: "nv-auth",
            section { class: "nv-auth__form",
            div { class: "nv-auth__card",
                div { class: "nv-auth__brand",
                    span { class: "nv-auth__logo", Icon { name: "logo", size: 28 } }
                    h1 { "{title}" }
                    p {
                        if register() {
                            "Ein Konto für Text, Sprache und Bildschirmfreigabe – in Sekunden."
                        } else {
                            "Melde dich an, um mit deinen Leuten weiterzureden."
                        }
                    }
                }
                form { class: "nv-auth__panel", onsubmit: submit, novalidate: true,
                    div { class: "nv-field",
                        label { class: "nv-field__label", r#for: "auth-handle", "Benutzername" }
                        input {
                            id: "auth-handle",
                            class: "nv-input",
                            autocomplete: "username",
                            autofocus: true,
                            spellcheck: "false",
                            placeholder: "z. B. mara",
                            "aria-invalid": invalid,
                            value: "{handle}",
                            oninput: move |event| handle.set(event.value()),
                        }
                        if register() {
                            span { class: "nv-field__hint", "3–32 Zeichen: a–z, 0–9, Punkt, Unterstrich." }
                        }
                    }
                    div { class: "nv-field",
                        label { class: "nv-field__label", r#for: "auth-password", "Passwort" }
                        input {
                            id: "auth-password",
                            class: "nv-input",
                            r#type: "password",
                            autocomplete: if register() { "new-password" } else { "current-password" },
                            "aria-invalid": invalid,
                            value: "{password}",
                            oninput: move |event| password.set(event.value()),
                        }
                        if register() {
                            span { class: "nv-field__hint", "Mindestens 10 Zeichen." }
                        }
                    }
                    if let Some(text) = error() {
                        p { class: "nv-field__error", role: "alert",
                            Icon { name: "alert", size: 16 }
                            "{text}"
                        }
                    }
                    button {
                        class: if busy() { "nv-btn nv-btn--primary nv-btn--block is-loading" } else { "nv-btn nv-btn--primary nv-btn--block" },
                        r#type: "submit",
                        disabled: busy(),
                        if busy() { "Einen Moment …" } else if register() { "Konto erstellen" } else { "Anmelden" }
                    }
                }
                p { class: "nv-auth__switch",
                    if register() { "Schon ein Konto? " } else { "Neu bei Nova? " }
                    button {
                        r#type: "button",
                        onclick: move |_| {
                            register.toggle();
                            error.set(None);
                        },
                        if register() { "Anmelden" } else { "Konto erstellen" }
                    }
                }
            }
            }
            LoungePreview {}
        }
    }
}

/// Statischer Ausschnitt einer Voice-Lounge: zeigt, worum es bei Nova geht.
#[component]
fn LoungePreview() -> Element {
    let people = [
        ("Mara Lenz", true, false),
        ("Jonas", false, false),
        ("Ida Krüger", false, false),
        ("Tom", false, true),
    ];
    rsx! {
        aside { class: "nv-auth__preview", "aria-hidden": "true",
            div { class: "nv-auth__lounge",
                div { class: "nv-auth__lounge-head", Icon { name: "volume", size: 18 } "Lounge" }
                div { class: "nv-stage__grid",
                    for (name, speaking, muted) in people {
                        div { key: "{name}", class: if speaking { "nv-tile is-speaking" } else { "nv-tile" },
                            Avatar { name: name.to_string(), size: 72, speaking }
                            span { class: "nv-tile__name", "{name}" }
                            if muted {
                                div { class: "nv-tile__icons", span { Icon { name: "mic-off", size: 14 } } }
                            }
                        }
                    }
                }
            }
        }
    }
}
