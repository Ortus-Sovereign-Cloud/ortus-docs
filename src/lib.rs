//! Docs: the shared editor, comments and suggestions, review, Word and PDF.
//!
//! An Ortus app crate: it depends on `ortus-core` only and hands its apps
//! to a program through [`apps`].

pub mod api;
pub mod review;
pub mod store;

use std::sync::Arc;

use ortus_core::app::{App, AppInfo, Group};
use ortus_core::instance::Instance;
use ortus_core::storage::{AsyncTx, BoxFuture, Migration};

/// The built web client (web/dist).
#[derive(rust_embed::RustEmbed)]
#[folder = "web/dist"]
struct Web;

/// The docs app.
pub struct Docs;

/// A document's name and text in the search index (under its owner).
pub async fn index(instance: &Instance, doc: &str, someone: &str, text: &str) {
    let Ok(drive) = instance
        .services
        .get::<dyn ortus_core::interfaces::Drive>(&instance.switches)
        .await
    else {
        return;
    };
    let Ok(Some(item)) = drive.describe(someone, doc).await else {
        return;
    };
    let entry = ortus_core::search::Entry {
        app: "docs",
        id: doc,
        owner: &item.owner,
        title: &item.name,
        body: text,
        meta: serde_json::json!({"path": format!("/docs?doc={doc}"), "kind": "document"}),
        at: item.modified_at,
    };
    let _ = ortus_core::search::index(instance.store.as_ref(), &entry).await;
}

impl App for Docs {
    fn info(&self) -> AppInfo {
        AppInfo {
            id: "docs",
            group: Group::Create,
            path: "/docs",
            order: 2,
        }
    }

    fn migrations(&self) -> &'static [Migration] {
        store::MIGRATIONS
    }

    fn routes(&self) -> Vec<ortus_core::api::Route> {
        let mut all = api::routes();
        all.extend(review::routes());
        all
    }

    fn search_owners<'a>(
        &'a self,
        _instance: &'a Instance,
        _user: &'a str,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async { Vec::new() })
    }

    fn forget_user<'a>(
        &'a self,
        tx: &'a mut dyn AsyncTx,
        user_id: &'a str,
        _rules: &'a ortus_core::retention::Rules,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { store::forget(tx, user_id).await.map_err(|e| e.to_string()) })
    }

    fn web_file(&self, path: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
        Web::get(path).map(|file| file.data)
    }
}

/// The apps of this crate, for a program to run.
pub fn apps() -> Vec<Arc<dyn App>> {
    vec![Arc::new(Docs)]
}
