//! The core with this crate's apps, for development and the browser tests
//! (`cargo run --example dev`). The released programs are built in the
//! `ortus` repository, with the real drive; here a stand-in drive keeps
//! documents in memory and lets every colleague edit them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ortus_core::app::{App, AppInfo, Group};
use ortus_core::instance::Instance;
use ortus_core::interfaces::{Drive, DriveItem, ItemRef};
use ortus_core::storage::BoxFuture;

#[derive(Default)]
struct StandIn(Mutex<HashMap<String, (String, String)>>);

impl Drive for StandIn {
    fn save<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a str,
        _: &'a [u8],
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<ItemRef, String>> {
        Box::pin(async { Err("the stand-in drive keeps documents only".into()) })
    }
    fn create<'a>(
        &'a self,
        user: &'a str,
        _: &'a str,
        name: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<ItemRef, String>> {
        let id = format!("d{}", ortus_core::auth::random_hex(8));
        self.0
            .lock()
            .unwrap()
            .insert(id.clone(), (user.to_string(), name.to_string()));
        Box::pin(async move {
            Ok(ItemRef {
                app: "drive".into(),
                id: id.clone(),
                path: format!("/docs?doc={id}"),
            })
        })
    }
    fn describe<'a>(
        &'a self,
        user: &'a str,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<DriveItem>, String>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .get(id)
            .map(|(owner, name)| DriveItem {
                id: id.into(),
                name: name.clone(),
                mime: "application/vnd.ortus.doc".into(),
                owner: owner.clone(),
                role: if owner == user {
                    "owner".into()
                } else {
                    "edit".into()
                },
                label: Some("official-sensitive".into()),
                path: vec!["Emergency".into(), "Plans".into()],
                modified_at: 0,
            });
        Box::pin(async move { Ok(found) })
    }
    fn rename<'a>(
        &'a self,
        _: &'a str,
        id: &'a str,
        name: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        if let Some(item) = self.0.lock().unwrap().get_mut(id) {
            item.1 = name.to_string();
        }
        Box::pin(async { Ok(()) })
    }
}

struct DevDrive(Arc<StandIn>);

impl App for DevDrive {
    fn info(&self) -> AppInfo {
        AppInfo {
            id: "drive",
            group: Group::Create,
            path: "/drive",
            order: 1,
        }
    }
    fn provide(&self, _: &Arc<Instance>, services: &ortus_core::services::Services) {
        let drive: Arc<dyn Drive> = self.0.clone();
        services.provide("drive", drive);
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut apps = ortus_docs::apps();
    apps.push(Arc::new(DevDrive(Arc::new(StandIn::default()))));
    let status = ortus_core::cli::run("ortus-docs-dev", ortus_core::VERSION, apps, &args).await;
    std::process::exit(status);
}
