//! Docs: the shared editor, comments and suggestions, review, Word and PDF.
//!
//! An Ortus app crate: it depends on `ortus-core` only and hands its apps
//! to a program through [`apps`].

pub mod api;
pub mod pdf;
pub mod review;
pub mod store;
pub mod ttf;

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

/// Hourly: documents the drive no longer has (deleted for good, or their
/// owner's account gone) are forgotten with all they kept.
pub const SWEEP_JOB: &str = "docs.sweep";

pub async fn sweep(instance: &Instance) -> Result<(), String> {
    let Ok(drive) = instance
        .services
        .get::<dyn ortus_core::interfaces::Drive>(&instance.switches)
        .await
    else {
        return Ok(());
    };
    let s = instance.store.as_ref();
    // Routes that finished while the records app could not take their PDF.
    let unfiled: Vec<String> = s
        .query("SELECT id FROM docs_reviews WHERE state = 'approved' AND pdf_key IS NOT NULL AND record_path IS NULL", &[])
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .filter_map(|r| r.text(0).map(str::to_string))
        .collect();
    for review in unfiled {
        let _ = review::file_record(instance, &review).await;
    }
    for (doc, owner) in store::documents(s).await.map_err(|e| e.to_string())? {
        if let Ok(None) = drive.describe(&owner, &doc).await {
            let mut tx = s.begin().await.map_err(|e| e.to_string())?;
            store::forget_document(tx.as_mut(), &doc)
                .await
                .map_err(|e| e.to_string())?;
            ortus_core::search::remove_in(tx.as_mut(), s.engine(), "docs", &doc)
                .await
                .map_err(|e| e.to_string())?;
            tx.commit().await.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
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

    // Documents are Drive items: whoever Drive lets see one finds it.
    fn search_owners<'a>(
        &'a self,
        instance: &'a Instance,
        user: &'a str,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            match instance.app("drive") {
                Some(drive) => drive.search_owners(instance, user).await,
                None => Vec::new(),
            }
        })
    }

    fn search_allowed<'a>(
        &'a self,
        instance: &'a Instance,
        user: &'a str,
        ids: Vec<String>,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            let Ok(drive) = instance
                .services
                .get::<dyn ortus_core::interfaces::Drive>(&instance.switches)
                .await
            else {
                return Vec::new();
            };
            let mut seen = Vec::new();
            for id in ids {
                if let Ok(Some(_)) = drive.describe(user, &id).await {
                    seen.push(id);
                }
            }
            seen
        })
    }

    fn forget_user<'a>(
        &'a self,
        tx: &'a mut dyn AsyncTx,
        user_id: &'a str,
        _rules: &'a ortus_core::retention::Rules,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { store::forget(tx, user_id).await.map_err(|e| e.to_string()) })
    }

    fn periodic(&self) -> Vec<(&'static str, i64)> {
        vec![(SWEEP_JOB, 60 * 60 * 1000)]
    }

    fn outside_jobs(&self, instance: &Arc<Instance>) -> Vec<ortus_core::app::OutsideJobKind> {
        let instance = Arc::clone(instance);
        vec![ortus_core::app::OutsideJobKind {
            kind: SWEEP_JOB.into(),
            handler: Box::new(move |_| {
                let instance = Arc::clone(&instance);
                Box::pin(async move { sweep(&instance).await })
            }),
        }]
    }

    fn web_file(&self, path: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
        Web::get(path).map(|file| file.data)
    }
}

/// The apps of this crate, for a program to run.
pub fn apps() -> Vec<Arc<dyn App>> {
    vec![Arc::new(Docs)]
}
