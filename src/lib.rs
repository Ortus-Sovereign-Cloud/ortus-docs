//! Docs: the shared editor, comments and suggestions, review, Word and PDF.
//!
//! An Ortus app crate: it depends on `ortus-core` only and hands its apps
//! to a program through [`apps`].

use std::sync::Arc;

use ortus_core::app::{App, AppInfo, Group};

/// The docs app.
pub struct Docs;

impl App for Docs {
    fn info(&self) -> AppInfo {
        AppInfo {
            id: "docs",
            group: Group::Create,
            path: "/docs",
            order: 2,
        }
    }
}

/// The apps of this crate, for a program to run.
pub fn apps() -> Vec<Arc<dyn App>> {
    vec![Arc::new(Docs)]
}
