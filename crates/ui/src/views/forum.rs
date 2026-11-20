//! Forum-Kanal: Beiträge als Liste, „Neuer Beitrag“; ein Klick öffnet den
//! Beitrag als Thread in der rechten Spalte.

use dioxus::prelude::*;

use crate::{
    actions, api,
    components::{EmptyState, Icon, Skeleton, classes, day_label, time_label},
    store::{Overlay, use_store},
    types::*,
};

#[component]
pub fn ForumView(channel: Channel) -> Element {
    let mut store = use_store();
    let can_post = store.can(perm::SEND_MESSAGES);
    let forum_id = channel.id.clone();
    let open_thread = store.thread_id.read().clone();
    // Neue Beiträge legen Threads an (`guild_update`): mit der Guild neu laden.
    let mut posts = use_resource(use_reactive!(|forum_id| async move {
        let _ = store
            .detail
            .read()
            .as_ref()
            .map(|detail| detail.channels.len());
        api::forum_posts(&forum_id).await
    }));
    let new_post = {
        let id = channel.id.clone();
        move |_| store.overlay.set(Overlay::NewPost(id.clone()))
    };
    let intro = channel
        .topic
        .clone()
        .filter(|topic| !topic.is_empty())
        .unwrap_or_else(|| "Jede Frage, jede Idee bekommt hier ihren eigenen Thread.".into());

    let list = match &*posts.read() {
        None => rsx! { Skeleton { lines: 5 } },
        Some(Err(err)) => rsx! {
            EmptyState {
                icon: "alert",
                title: "Beiträge konnten nicht geladen werden",
                text: "{err}",
                button { class: "nv-btn nv-btn--primary", onclick: move |_| posts.restart(), Icon { name: "refresh", size: 16 } "Erneut versuchen" }
            }
        },
        Some(Ok(list)) if list.is_empty() => rsx! {
            EmptyState {
                icon: "forum",
                title: "Noch keine Beiträge",
                text: if can_post { "Starte den ersten – die Antworten landen gesammelt darunter." } else { "Hier hat noch niemand etwas geschrieben." },
                if can_post {
                    button { class: "nv-btn nv-btn--primary", onclick: new_post.clone(), Icon { name: "plus", size: 16 } "Neuer Beitrag" }
                }
            }
        },
        Some(Ok(list)) => {
            let list = list.clone();
            rsx! {
                ul { class: "nv-posts", "aria-label": "Beiträge in {channel.name}",
                    for post in list {
                        {
                            let id = post.id.clone();
                            let active = open_thread.as_deref() == Some(post.id.as_str());
                            let author = post.author_name.clone().unwrap_or_else(|| "Unbekannt".into());
                            let when = format!("{}, {}", day_label(&post.created_at), time_label(&post.created_at));
                            let replies = match post.replies {
                                0 => "Noch keine Antwort".to_owned(),
                                1 => "1 Antwort".to_owned(),
                                n => format!("{n} Antworten"),
                            };
                            rsx! {
                                li { key: "{post.id}",
                                    button {
                                        class: classes(&["nv-post", if active { "is-active" } else { "" }]),
                                        "aria-current": if active { "true" } else { "false" },
                                        onclick: move |_| { spawn(actions::open_thread(store, id.clone())); },
                                        span { class: "nv-post__title",
                                            "{post.title}"
                                            if post.archived {
                                                span { class: "nv-badge", "archiviert" }
                                            }
                                        }
                                        if let Some(preview) = post.preview.clone().filter(|p| !p.is_empty()) {
                                            span { class: "nv-post__preview", "{preview}" }
                                        }
                                        span { class: "nv-post__meta",
                                            span { "{author}" }
                                            span { "{when}" }
                                            span { class: "nv-post__replies", Icon { name: "thread", size: 14 } "{replies}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "nv-forum",
            div { class: "nv-forum__bar",
                p { class: "nv-muted", "{intro}" }
                if can_post {
                    button { class: "nv-btn nv-btn--primary", onclick: new_post, Icon { name: "plus", size: 16 } "Neuer Beitrag" }
                }
            }
            {list}
        }
    }
}
