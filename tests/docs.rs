//! Docs on both installation shapes, with a stand-in drive: two people
//! edit one document (their updates are kept in order and handed to
//! every editor; a checkpoint folds them in and its text goes to search), comments, named versions, and the
//! review route (in order, approve, send back with a comment). Nobody
//! reaches a document the drive does not let them see.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use ortus_core::app::{App, AppInfo, Group};
use ortus_core::auth::accounts::Role;
use ortus_core::instance::{testing_auth::signed_in, Instance};
use ortus_core::interfaces::{Drive, DriveItem, ItemRef};
use ortus_core::storage::BoxFuture;
use rust_web_server::test_client::TestClient;
use serde_json::{json, Value};

/// An item: owner, name, roles by user.
type Entry = (String, String, HashMap<String, String>);

#[derive(Default)]
struct Items(Mutex<HashMap<String, Entry>>);

impl Drive for Items {
    fn save<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a str,
        _: &'a [u8],
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<ItemRef, String>> {
        Box::pin(async { Err("no".into()) })
    }
    fn create<'a>(
        &'a self,
        user: &'a str,
        _: &'a str,
        name: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<ItemRef, String>> {
        let id = format!("d{}", self.0.lock().unwrap().len() + 1);
        self.0.lock().unwrap().insert(
            id.clone(),
            (
                user.to_string(),
                name.to_string(),
                HashMap::from([(user.to_string(), "owner".to_string())]),
            ),
        );
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
            .and_then(|(owner, name, roles)| {
                roles.get(user).map(|role| DriveItem {
                    id: id.into(),
                    name: name.clone(),
                    mime: "application/vnd.ortus.doc".into(),
                    owner: owner.clone(),
                    role: role.clone(),
                    label: Some("official-sensitive".into()),
                    path: vec!["Plans".into()],
                    modified_at: 1,
                })
            });
        Box::pin(async move { Ok(found) })
    }
    fn rename<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

struct FakeDrive(Arc<Items>);

impl App for FakeDrive {
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
    // The owners of items shared with the user, as the real drive answers.
    fn search_owners<'a>(&'a self, _: &'a Instance, user: &'a str) -> BoxFuture<'a, Vec<String>> {
        let owners = self
            .0
             .0
            .lock()
            .unwrap()
            .values()
            .filter(|(owner, _, roles)| owner != user && roles.contains_key(user))
            .map(|(owner, _, _)| owner.clone())
            .collect();
        Box::pin(async move { owners })
    }
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn call<A: rust_web_server::application::Application>(
    client: &TestClient<A>,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Vec<u8>>,
    json_body: bool,
) -> (i16, Value) {
    let url = format!("/api/v1{path}");
    let mut r = match method {
        "GET" => client.get(&url),
        "PATCH" => client.patch(&url),
        _ => client.post(&url),
    };
    r = r
        .header("Authorization", &format!("Bearer {token}"))
        .header("X-Requested-With", "ortus");
    if let Some(b) = body {
        r = r
            .header(
                "Content-Type",
                if json_body {
                    "application/json"
                } else {
                    "application/octet-stream"
                },
            )
            .body_bytes(b);
    }
    let response = r.send();
    (
        response.status(),
        serde_json::from_slice(response.body_bytes()).unwrap_or(Value::Null),
    )
}

#[test]
fn two_people_edit_one_document_and_send_it_for_approval() {
    for (name, base, _keep) in common::installations() {
        let items = Arc::new(Items::default());
        let mut apps = ortus_docs::apps();
        apps.push(Arc::new(FakeDrive(Arc::clone(&items))));
        let instance = ortus_core::instance::testing_store::instance(Arc::clone(&base.store), apps);
        for app in instance.apps.clone() {
            app.provide(&instance, &instance.services);
        }
        let client = TestClient::new(ortus_core::app(Arc::clone(&instance)));
        let marek = signed_in(&instance, "marek@ministry.example", Role::Member);
        let ana = signed_in(&instance, "ana@ministry.example", Role::Member);
        let olena = signed_in(&instance, "olena@ministry.example", Role::Member);
        let sam = signed_in(&instance, "sam@ministry.example", Role::Member);
        let id_of = |e: &str| {
            ortus_core::runtime::block_on(ortus_core::auth::accounts::by_email(
                instance.store.as_ref(),
                e,
            ))
            .unwrap()
            .unwrap()
            .id
        };

        let (status, made) = call(
            &client,
            &marek,
            "POST",
            "/docs/documents",
            Some(
                json!({"name": "Flood response plan 2026"})
                    .to_string()
                    .into_bytes(),
            ),
            true,
        );
        assert_eq!(200, status, "{name}: {made}");
        let doc = made["id"].as_str().unwrap().to_string();
        // Ana may edit, Olena may view (she reviews), Sam nothing.
        items.0.lock().unwrap().get_mut(&doc).unwrap().2.extend([
            (id_of("ana@ministry.example"), "edit".to_string()),
            (id_of("olena@ministry.example"), "view".to_string()),
        ]);

        // Two editors write (updates are opaque to the server).
        assert_eq!(
            200,
            call(
                &client,
                &marek,
                "POST",
                &format!("/docs/documents/{doc}/updates?client=m1"),
                Some(b"update-one".to_vec()),
                false
            )
            .0
        );
        assert_eq!(
            200,
            call(
                &client,
                &ana,
                "POST",
                &format!("/docs/documents/{doc}/updates?client=a1"),
                Some(b"update-two".to_vec()),
                false
            )
            .0
        );
        assert_eq!(
            400,
            call(
                &client,
                &ana,
                "POST",
                &format!("/docs/documents/{doc}/updates?client=a1"),
                Some(Vec::new()),
                false
            )
            .0,
            "{name}"
        );
        assert_eq!(
            404,
            call(
                &client,
                &sam,
                "GET",
                &format!("/docs/documents/{doc}"),
                None,
                false
            )
            .0,
            "{name}: not his"
        );
        assert_eq!(
            403,
            call(
                &client,
                &olena,
                "POST",
                &format!("/docs/documents/{doc}/updates?client=o1"),
                Some(b"x".to_vec()),
                false
            )
            .0,
            "{name}: view only"
        );

        // Opening it gives every update, in order.
        let (_, opened) = call(
            &client,
            &olena,
            "GET",
            &format!("/docs/documents/{doc}"),
            None,
            false,
        );
        assert_eq!(
            (
                json!("official-sensitive"),
                json!("view"),
                json!(2),
                Value::Null
            ),
            (
                opened["label"].clone(),
                opened["role"].clone(),
                opened["seq"].clone(),
                opened["base"].clone()
            ),
            "{name}"
        );
        let updates: Vec<Vec<u8>> = opened["updates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| B64.decode(u.as_str().unwrap()).unwrap())
            .collect();
        assert_eq!(
            vec![b"update-one".to_vec(), b"update-two".to_vec()],
            updates,
            "{name}"
        );

        // A checkpoint folds them in; its text goes to search.
        let structure = json!({"type": "doc", "content": [{"type": "heading", "content": [{"type": "text", "text": "Flood response plan 2026"}]}, {"type": "paragraph", "content": [{"type": "text", "text": "Each district opens at least one shelter."}]}]});
        let (status, cp) = call(
            &client,
            &ana,
            "POST",
            &format!("/docs/documents/{doc}/checkpoint"),
            Some(
                json!({"seq": 2, "state": B64.encode(b"merged"), "doc": structure})
                    .to_string()
                    .into_bytes(),
            ),
            true,
        );
        assert_eq!(
            (200, json!(true)),
            (status, cp["taken"].clone()),
            "{name}: {cp}"
        );
        let (_, stale) = call(
            &client,
            &ana,
            "POST",
            &format!("/docs/documents/{doc}/checkpoint"),
            Some(
                json!({"seq": 1, "state": B64.encode(b"older"), "doc": {}})
                    .to_string()
                    .into_bytes(),
            ),
            true,
        );
        assert_eq!(
            json!(false),
            stale["taken"],
            "{name}: an older checkpoint is ignored"
        );
        let (_, again) = call(
            &client,
            &marek,
            "GET",
            &format!("/docs/documents/{doc}"),
            None,
            false,
        );
        assert_eq!(
            (json!(B64.encode(b"merged")), 0, json!(2)),
            (
                again["base"].clone(),
                again["updates"].as_array().unwrap().len(),
                again["seq"].clone()
            ),
            "{name}"
        );
        let hits = ortus_core::runtime::block_on(ortus_core::search::search(
            &instance,
            &id_of("marek@ministry.example"),
            &ortus_core::search::Query::parse("shelter"),
            10,
        ))
        .unwrap();
        assert_eq!(
            vec!["Flood response plan 2026".to_string()],
            hits.iter().map(|h| h.title.clone()).collect::<Vec<_>>(),
            "{name}"
        );
        // Olena, who may view it, finds it too; someone it is not shared
        // with does not.
        let titles = |who: &str| {
            ortus_core::runtime::block_on(ortus_core::search::search(
                &instance,
                &id_of(who),
                &ortus_core::search::Query::parse("shelter"),
                10,
            ))
            .unwrap()
            .into_iter()
            .map(|h| h.title)
            .collect::<Vec<_>>()
        };
        assert_eq!(
            vec!["Flood response plan 2026".to_string()],
            titles("olena@ministry.example"),
            "{name}"
        );
        signed_in(&instance, "jonas@ministry.example", Role::Member);
        assert!(titles("jonas@ministry.example").is_empty(), "{name}");

        // Comments, assigned.
        let (status, c) = call(&client, &marek, "POST", &format!("/docs/documents/{doc}/comments"), Some(json!({"body": "Hydrology says 3.8 m is safer. @Anna can you confirm?", "quote": "4.0 m", "assign": "ana@ministry.example"}).to_string().into_bytes()), true);
        assert_eq!(200, status, "{name}: {c}");
        let thread = c["thread"].as_str().unwrap().to_string();
        call(
            &client,
            &ana,
            "POST",
            &format!("/docs/documents/{doc}/comments"),
            Some(
                json!({"thread": thread, "body": "Confirmed."})
                    .to_string()
                    .into_bytes(),
            ),
            true,
        );
        let (_, list) = call(
            &client,
            &olena,
            "GET",
            &format!("/docs/documents/{doc}/comments"),
            None,
            false,
        );
        assert_eq!(2, list["comments"].as_array().unwrap().len());
        assert_eq!(
            json!("ana@ministry.example"),
            list["comments"][0]["assignedTo"],
            "{name}"
        );

        // The review route: Olena, then Ana. Olena approves; Ana sends back.
        let (status, review) = call(&client, &marek, "POST", &format!("/docs/documents/{doc}/reviews"), Some(json!({"steps": [{"email": "olena@ministry.example", "title": "Logistics review"}, {"email": "ana@ministry.example", "title": "Director signs"}]}).to_string().into_bytes()), true);
        assert_eq!(200, status, "{name}: {review}");
        let rid = review["id"].as_str().unwrap().to_string();
        let (_, waiting) = call(&client, &olena, "GET", "/docs/reviews/mine", None, false);
        assert_eq!(
            json!("Flood response plan 2026"),
            waiting["waiting"][0]["name"],
            "{name}"
        );
        assert_eq!(
            403,
            call(
                &client,
                &ana,
                "POST",
                &format!("/docs/reviews/{rid}/decide"),
                Some(json!({"approve": true}).to_string().into_bytes()),
                true
            )
            .0,
            "{name}: not her turn"
        );
        let (_, after) = call(
            &client,
            &olena,
            "POST",
            &format!("/docs/reviews/{rid}/decide"),
            Some(json!({"approve": true}).to_string().into_bytes()),
            true,
        );
        assert_eq!(
            (json!("approved"), json!("current")),
            (
                after["steps"][0]["state"].clone(),
                after["steps"][1]["state"].clone()
            ),
            "{name}"
        );
        assert_eq!(
            400,
            call(
                &client,
                &ana,
                "POST",
                &format!("/docs/reviews/{rid}/decide"),
                Some(json!({"approve": false}).to_string().into_bytes()),
                true
            )
            .0,
            "{name}: a reason to send back"
        );
        let (_, back) = call(
            &client,
            &ana,
            "POST",
            &format!("/docs/reviews/{rid}/decide"),
            Some(
                json!({"approve": false, "comment": "Use 3.8 m."})
                    .to_string()
                    .into_bytes(),
            ),
            true,
        );
        assert_eq!(json!("returned"), back["state"], "{name}");
        // Sam cannot be a reviewer: he cannot open it.
        assert_eq!(
            400,
            call(
                &client,
                &marek,
                "POST",
                &format!("/docs/documents/{doc}/reviews"),
                Some(
                    json!({"steps": [{"email": "sam@ministry.example"}]})
                        .to_string()
                        .into_bytes()
                ),
                true
            )
            .0,
            "{name}"
        );
    }
}

/// The slice review's findings: commenters cannot checkpoint; a step is
/// decided once; a route skips people who left and can be withdrawn;
/// reviewers see the version that was sent; documents the drive no
/// longer has are swept away; a reader too far behind is told to reload.
#[test]
fn routes_hold_under_races_departures_and_edits() {
    for (name, base, _keep) in common::installations() {
        let items = Arc::new(Items::default());
        let mut apps = ortus_docs::apps();
        apps.push(Arc::new(FakeDrive(Arc::clone(&items))));
        let instance = ortus_core::instance::testing_store::instance(Arc::clone(&base.store), apps);
        for app in instance.apps.clone() {
            app.provide(&instance, &instance.services);
        }
        let client = TestClient::new(ortus_core::app(Arc::clone(&instance)));
        let marek = signed_in(&instance, "marek@ministry.example", Role::Member);
        let ana = signed_in(&instance, "ana@ministry.example", Role::Member);
        let olena = signed_in(&instance, "olena@ministry.example", Role::Member);
        signed_in(&instance, "sam@ministry.example", Role::Member);
        let id_of = |e: &str| {
            ortus_core::runtime::block_on(ortus_core::auth::accounts::by_email(
                instance.store.as_ref(),
                e,
            ))
            .unwrap()
            .unwrap()
            .id
        };
        let post = |token: &str, path: &str, v: Value| {
            call(
                &client,
                token,
                "POST",
                path,
                Some(v.to_string().into_bytes()),
                true,
            )
        };
        let (_, made) = post(&marek, "/docs/documents", json!({"name": "Shelter rota"}));
        let doc = made["id"].as_str().unwrap().to_string();
        items.0.lock().unwrap().get_mut(&doc).unwrap().2.extend([
            (id_of("ana@ministry.example"), "comment".to_string()),
            (id_of("olena@ministry.example"), "view".to_string()),
        ]);
        let first = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Rota as sent."}]}]});
        call(
            &client,
            &marek,
            "POST",
            &format!("/docs/documents/{doc}/updates?client=m"),
            Some(b"u1".to_vec()),
            false,
        );
        // A commenter may send updates, not checkpoints.
        let (status, _) = post(
            &ana,
            &format!("/docs/documents/{doc}/checkpoint"),
            json!({"seq": 1, "state": B64.encode(b"x"), "doc": first}),
        );
        assert_eq!(403, status, "{name}");
        let (status, _) = post(
            &marek,
            &format!("/docs/documents/{doc}/checkpoint"),
            json!({"seq": 1, "state": B64.encode(b"s1"), "doc": first}),
        );
        assert_eq!(200, status, "{name}");

        // Olena, then Ana; Olena's second click finds her step decided.
        let (status, review) = post(
            &marek,
            &format!("/docs/documents/{doc}/reviews"),
            json!({"steps": [{"email": "olena@ministry.example"}, {"email": "ana@ministry.example"}]}),
        );
        assert_eq!(200, status, "{name}: {review}");
        let r = review["id"].as_str().unwrap().to_string();
        // Edited after sending: the reviewer still sees what was sent.
        call(
            &client,
            &marek,
            "POST",
            &format!("/docs/documents/{doc}/updates?client=m"),
            Some(b"u2".to_vec()),
            false,
        );
        let later = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Rota changed later."}]}]});
        post(
            &marek,
            &format!("/docs/documents/{doc}/checkpoint"),
            json!({"seq": 2, "state": B64.encode(b"s2"), "doc": later}),
        );
        let (_, sent) = call(
            &client,
            &olena,
            "GET",
            &format!("/docs/reviews/{r}/document"),
            None,
            false,
        );
        assert_eq!(
            (json!(first), json!(true)),
            (sent["doc"].clone(), sent["edited"].clone()),
            "{name}: {sent}"
        );
        assert_eq!(
            200,
            post(
                &olena,
                &format!("/docs/reviews/{r}/decide"),
                json!({"approve": true})
            )
            .0,
            "{name}"
        );
        assert_eq!(
            403,
            post(
                &olena,
                &format!("/docs/reviews/{r}/decide"),
                json!({"approve": true})
            )
            .0,
            "{name}"
        );

        // Ana leaves while it is her turn: nobody is left, so it is approved.
        let ana_id = id_of("ana@ministry.example");
        ortus_core::runtime::block_on(async {
            let mut tx = instance.store.begin().await.unwrap();
            for app in instance.apps.iter() {
                app.forget_user(
                    tx.as_mut(),
                    &ana_id,
                    &ortus_core::retention::Rules::default(),
                )
                .await
                .unwrap();
            }
            tx.commit().await.unwrap();
        });
        let (_, after) = call(
            &client,
            &marek,
            "GET",
            &format!("/docs/reviews/{r}"),
            None,
            false,
        );
        assert_eq!(json!("approved"), after["state"], "{name}: {after}");
        assert_eq!(
            json!("skipped"),
            after["steps"][1]["state"],
            "{name}: {after}"
        );

        // A new route is withdrawn; its reviewer can no longer decide.
        let (_, again) = post(
            &marek,
            &format!("/docs/documents/{doc}/reviews"),
            json!({"steps": [{"email": "olena@ministry.example"}]}),
        );
        let r2 = again["id"].as_str().unwrap().to_string();
        assert_eq!(
            403,
            post(&olena, &format!("/docs/reviews/{r2}/cancel"), json!({})).0,
            "{name}"
        );
        let (status, gone) = post(&marek, &format!("/docs/reviews/{r2}/cancel"), json!({}));
        assert_eq!(
            (200, json!("cancelled")),
            (status, gone["state"].clone()),
            "{name}: {gone}"
        );
        assert_ne!(
            200,
            post(
                &olena,
                &format!("/docs/reviews/{r2}/decide"),
                json!({"approve": true})
            )
            .0,
            "{name}"
        );
        // Strangers' addresses and people without access read alike.
        let (_, nobody) = post(
            &marek,
            &format!("/docs/documents/{doc}/reviews"),
            json!({"steps": [{"email": "nobody@ministry.example"}]}),
        );
        let (_, outsider) = post(
            &marek,
            &format!("/docs/documents/{doc}/reviews"),
            json!({"steps": [{"email": "sam@ministry.example"}]}),
        );
        assert!(
            nobody.to_string().contains("cannot open"),
            "{name}: {nobody}"
        );
        assert!(
            outsider.to_string().contains("cannot open"),
            "{name}: {outsider}"
        );

        // The drive deletes it for good: the sweep forgets what docs kept.
        items.0.lock().unwrap().remove(&doc);
        ortus_core::runtime::block_on(ortus_docs::sweep(&instance)).unwrap();
        let left = ortus_core::runtime::block_on(instance.store.query(
            "SELECT COUNT(*) FROM docs_snapshots WHERE doc_id = $1",
            &[doc.as_str().into()],
        ))
        .unwrap();
        assert_eq!(Some(0), left[0].int(0), "{name}");
    }
}
