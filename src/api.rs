//! The docs API: documents (made in the drive), their sync (updates in by
//! POST, out by server-sent events, so it works on every node of a
//! cluster), presence, comments, named versions and the review route.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use ortus_core::api::{answer, body, ok, ApiError, Method, Route};
use ortus_core::auth::Access;
use ortus_core::instance::Instance;
use ortus_core::interfaces::{Drive, DriveItem};
use ortus_core::storage::BoxFuture;
use rust_web_server::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::store;

/// The drive's type for documents.
pub const DOC_MIME: &str = "application/vnd.ortus.doc";

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn bytes_len(request: &Request) -> i64 {
    request.body.as_bytes().len() as i64
}

pub(crate) fn query(request: &Request, name: &str) -> Option<String> {
    let q = request.request_uri.split_once('?')?.1;
    q.split('&').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k == name).then(|| ortus_core::form::decode(v))
    })
}

pub(crate) async fn signed(
    request: &Request,
    instance: &Arc<Instance>,
) -> Result<ortus_core::auth::accounts::User, ApiError> {
    Ok(ortus_core::auth::current(request, instance)
        .await
        .ok_or_else(ApiError::sign_in)?
        .user)
}

pub(crate) async fn drive(instance: &Instance) -> Result<Arc<dyn Drive>, ApiError> {
    instance
        .services
        .get::<dyn Drive>(&instance.switches)
        .await
        .map_err(|_| ApiError::not_installed("drive"))
}

fn rank(role: &str) -> u8 {
    match role {
        "owner" => 4,
        "edit" => 3,
        "comment" => 2,
        "view" => 1,
        _ => 0,
    }
}

/// A document the person may reach with at least `need` (view, comment,
/// edit); others get "not found".
pub(crate) async fn document(
    instance: &Instance,
    user: &str,
    id: &str,
    need: &str,
) -> Result<DriveItem, ApiError> {
    let found = drive(instance)
        .await?
        .describe(user, id)
        .await
        .map_err(|_| ApiError::internal())?;
    let item = found
        .filter(|i| i.mime == DOC_MIME)
        .ok_or_else(|| ApiError::not_found("There is no such document."))?;
    if rank(&item.role) < rank(need) {
        return Err(ApiError::forbidden());
    }
    Ok(item)
}

async fn names(instance: &Instance, ids: &[String]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for id in ids {
        if !out.contains_key(id) {
            if let Ok(Some(u)) =
                ortus_core::auth::accounts::by_id(instance.store.as_ref(), id).await
            {
                out.insert(id.clone(), u.name);
            }
        }
    }
    out
}

#[derive(Deserialize)]
struct NewDoc {
    #[serde(default)]
    parent: String,
    name: String,
}

async fn create(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: NewDoc = body(request)?;
    let made = drive(instance)
        .await?
        .create(&me.id, &input.parent, &input.name, DOC_MIME)
        .await
        .map_err(|e| ApiError {
            status: 409,
            code: "drive",
            message: e,
        })?;
    // Kept to find documents the drive no longer has (the sweep), asked
    // as the drive item's owner (the folder's, in a shared folder).
    let drive_owner = drive(instance)
        .await?
        .describe(&me.id, &made.id)
        .await
        .ok()
        .flatten()
        .map(|item| item.owner);
    instance
        .store
        .execute(
            "INSERT INTO docs_documents (doc_id, owner_id, created_at, drive_owner) VALUES ($1, $2, $3, $4)",
            &[
                made.id.as_str().into(),
                me.id.as_str().into(),
                store::now().into(),
                drive_owner.into(),
            ],
        )
        .await?;
    Ok(ok(&json!({ "id": made.id, "path": made.path })))
}

/// A document with its whole state (to start the editor from).
async fn open(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "view",
    )
    .await?;
    let s = instance.store.as_ref();
    let (base, seq, after) = store::state(s, &item.id).await?;
    let seq = after.last().map_or(seq, |(n, _)| *n);
    let reviews = crate::review::of_document(instance, &item.id).await?;
    Ok(ok(&json!({
        "id": item.id,
        "name": item.name,
        "path": item.path,
        "label": item.label,
        "role": item.role,
        "base": base.map(|b| B64.encode(b)),
        "updates": after.iter().map(|(_, d)| B64.encode(d)).collect::<Vec<_>>(),
        "seq": seq,
        "me": {"id": me.id, "name": me.name},
        "review": reviews.first(),
    })))
}

async fn rename(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    #[derive(Deserialize)]
    struct Name {
        name: String,
    }
    let me = signed(request, instance).await?;
    let input: Name = body(request)?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "edit",
    )
    .await?;
    drive(instance)
        .await?
        .rename(&me.id, &item.id, &input.name)
        .await
        .map_err(|e| ApiError {
            status: 409,
            code: "drive",
            message: e,
        })?;
    Ok(ok(&json!({})))
}

/// An update from an editor (the body is a Yjs update). People who may
/// comment send updates too: comments and suggestions are marks in the
/// document; their editor writes nothing else.
async fn update(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "comment",
    )
    .await?;
    let bytes = request.body.as_bytes();
    if !store::valid_update(bytes) {
        return Err(ApiError::bad_request("That is not a document update."));
    }
    let client: String = query(request, "client")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(40)
        .collect();
    let s = instance.store.as_ref();
    if rank(&item.role) < rank("edit") {
        // Someone who may only comment: comments and suggestions in their
        // own name, nothing of the text (checked here, not only in their
        // editor).
        let (base, _, after) = store::state(s, &item.id).await?;
        let mut parts: Vec<&[u8]> = Vec::new();
        if let Some(b) = &base {
            parts.push(b);
        }
        parts.extend(after.iter().map(|(_, u)| u.as_slice()));
        crate::guard::commenter_may(&parts, bytes, &me.name).map_err(|message| ApiError {
            status: 403,
            code: "comment_only",
            message,
        })?;
    }
    let (loose, held) = store::loose(s, &item.id).await?;
    if loose >= store::MAX_LOOSE || held + bytes_len(request) > store::MAX_LOOSE_BYTES {
        return Err(ApiError {
            status: 409,
            code: "checkpoint",
            message: "The document needs a checkpoint first.".into(),
        });
    }
    // Two writers may race for the next number: the loser tries again.
    let mut tries = 0;
    let seq = loop {
        match store::add_update(s, &item.id, &me.id, &client, bytes).await {
            Ok(seq) => break seq,
            Err(_) if tries < 5 => tries += 1,
            Err(e) => return Err(e.into()),
        }
    };
    Ok(ok(&json!({ "seq": seq })))
}

/// The person's editor is here (its awareness state, base64).
async fn presence(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    #[derive(Deserialize)]
    struct Here {
        client: String,
        state: String,
    }
    let me = signed(request, instance).await?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "view",
    )
    .await?;
    let input: Here = body(request)?;
    let client: String = input
        .client
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(40)
        .collect();
    if client.is_empty() || input.state.len() > 16 * 1024 || B64.decode(&input.state).is_err() {
        return Err(ApiError::bad_request("That is not a presence."));
    }
    store::set_presence(
        instance.store.as_ref(),
        &item.id,
        &client,
        &me.id,
        &input.state,
    )
    .await?;
    Ok(ok(&json!({})))
}

/// Event streams open per person (all documents), at most.
const MAX_STREAMS: usize = 20;

static STREAMS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, usize>>> =
    std::sync::LazyLock::new(Default::default);

/// One open stream of a person; counted while it lives.
struct Open(String);

impl Open {
    fn take(user: &str) -> Option<Open> {
        let mut all = STREAMS.lock().unwrap_or_else(|e| e.into_inner());
        let n = all.entry(user.to_string()).or_insert(0);
        if *n >= MAX_STREAMS {
            return None;
        }
        *n += 1;
        Some(Open(user.to_string()))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let mut all = STREAMS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = all.get_mut(&self.0) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                all.remove(&self.0);
            }
        }
    }
}

/// Changes as they happen: `update` events (seq, data, client) after
/// ?since=, and `presence` events with who is here. A client that is
/// behind the kept checkpoint (its updates were folded in) gets `reload`
/// and starts again from the document's state.
fn events(
    request: &Request,
    params: &PathParams,
    instance: Arc<Instance>,
    user: String,
) -> Response {
    let since: i64 = query(request, "since")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let doc = params.get("doc").unwrap_or_default().to_string();
    let Some(open) = Open::take(&user) else {
        return answer(
            request,
            Err::<Response, _>(ApiError {
                status: 429,
                code: "streams",
                message: "Too many open documents.".into(),
            }),
        );
    };
    let (events, response) = rust_web_server::sse::SseStream::new()
        .keep_alive(Duration::from_secs(15))
        .start();
    ortus_core::runtime::io().spawn(async move {
        let _open = open;
        let mut seq = since;
        let mut seen_presence = String::new();
        let started = store::now();
        let mut checked = started;
        let mut pinged = started;
        loop {
            let s = instance.store.as_ref();
            // Access is looked at again every minute.
            if store::now() - checked > 60_000 {
                checked = store::now();
                if document(&instance, &user, &doc, "view").await.is_err() {
                    return;
                }
            }
            // A gone client is noticed within seconds.
            if store::now() - pinged > 2_000 {
                pinged = store::now();
                if events.comment("ping").is_err() {
                    return;
                }
            }
            match store::kept_seq(s, &doc).await {
                Ok(kept) if kept > seq => {
                    let _ = events.send_async(rust_web_server::sse::SseEvent::data(json!({"seq": kept}).to_string()).event_type("reload")).await;
                    return;
                }
                Ok(_) => {}
                Err(_) => return,
            }
            match store::updates_since(s, &doc, seq).await {
                Ok(list) => {
                    for (n, data, client) in list {
                        let event = rust_web_server::sse::SseEvent::data(json!({"seq": n, "data": B64.encode(&data), "client": client}).to_string()).event_type("update");
                        // Waits for room: nothing is dropped.
                        if events.send_async(event).await.is_err() {
                            return;
                        }
                        seq = n;
                    }
                }
                Err(_) => return,
            }
            if let Ok(here) = store::presence(s, &doc).await {
                let ids: Vec<String> = here.iter().map(|h| h.1.clone()).collect();
                let people = names(&instance, &ids).await;
                let list: Vec<Value> = here.iter().map(|(client, uid, state, _)| json!({"client": client, "user": uid, "name": people.get(uid), "state": state})).collect();
                let text = json!(list).to_string();
                if text != seen_presence {
                    seen_presence = text.clone();
                    if events.send_async(rust_web_server::sse::SseEvent::data(text).event_type("presence")).await.is_err() {
                        return;
                    }
                }
            }
            // A connection lasts ten minutes; the client comes back.
            if store::now() - started > 600_000 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
    response
}

fn h_events(
    request: Request,
    params: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move {
        let me = match signed(&request, &instance).await {
            Ok(me) => me,
            Err(e) => return answer(&request, Err::<Response, _>(e)),
        };
        match document(
            &instance,
            &me.id,
            params.get("doc").unwrap_or_default(),
            "view",
        )
        .await
        {
            Ok(_) => events(&request, &params, instance, me.id),
            Err(e) => answer(&request, Err::<Response, _>(e)),
        }
    })
}

async fn versions(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "view",
    )
    .await?;
    let rows = instance.store.query("SELECT n, name, created_by, created_at FROM docs_versions WHERE doc_id = $1 ORDER BY n DESC", &[item.id.as_str().into()]).await?;
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.text(2).map(str::to_string))
        .collect();
    let people = names(instance, &ids).await;
    let list: Vec<Value> = rows.iter().map(|r| json!({"n": r.int(0), "name": r.text(1), "by": people.get(r.text(2).unwrap_or_default()), "at": r.int(3)})).collect();
    Ok(ok(&json!({ "versions": list })))
}

async fn add_version(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    #[derive(Deserialize)]
    struct Named {
        #[serde(default)]
        name: String,
    }
    let me = signed(request, instance).await?;
    let input: Named = body(request)?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "edit",
    )
    .await?;
    let name: String = input
        .name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect();
    let n = store::add_version(instance.store.as_ref(), &item.id, &me.id, &name).await?;
    Ok(ok(&json!({ "n": n })))
}

/// The text of the document now and as it was in version ?since=, for
/// "changed since".
async fn changes(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "view",
    )
    .await?;
    let s = instance.store.as_ref();
    let since: i64 = query(request, "since")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let before = store::version_text(s, &item.id, since)
        .await?
        .unwrap_or_default();
    let (now, structure) = store::text(s, &item.id).await?;
    let doc: Value = serde_json::from_str(&structure).unwrap_or(Value::Null);
    Ok(ok(
        &json!({ "before": before, "now": now, "doc": doc, "name": item.name, "label": item.label }),
    ))
}

#[derive(Deserialize)]
struct Checkpoint {
    seq: i64,
    /// The whole document merged (a Yjs update, base64).
    state: String,
    /// Its structure (ProseMirror JSON), for text, search and export.
    doc: Value,
}

/// An editor's checkpoint of the whole document up to `seq`.
async fn checkpoint(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let item = document(
        instance,
        &me.id,
        params.get("doc").unwrap_or_default(),
        "edit",
    )
    .await?;
    let input: Checkpoint = body(request)?;
    let data = B64
        .decode(&input.state)
        .map_err(|_| ApiError::bad_request("The state is base64."))?;
    if data.is_empty()
        || data.len() > 32 * 1024 * 1024
        || input.doc.to_string().len() > 32 * 1024 * 1024
    {
        return Err(ApiError::bad_request("The checkpoint is too big."));
    }
    let s = instance.store.as_ref();
    let taken = store::checkpoint(s, &item.id, input.seq, &data, &input.doc).await?;
    if taken {
        let (text, _) = store::text(s, &item.id).await?;
        crate::index(instance, &item.id, &me.id, &text).await;
    }
    Ok(ok(&json!({ "taken": taken })))
}

macro_rules! plain {
    ($name:ident, $f:ident) => {
        fn $name(
            request: Request,
            _: PathParams,
            _: ConnectionInfo,
            instance: Arc<Instance>,
        ) -> BoxFuture<'static, Response> {
            Box::pin(async move { answer(&request, $f(&request, &instance).await) })
        }
    };
}
macro_rules! with_params {
    ($name:ident, $f:ident) => {
        fn $name(
            request: Request,
            params: PathParams,
            _: ConnectionInfo,
            instance: Arc<Instance>,
        ) -> BoxFuture<'static, Response> {
            Box::pin(async move { answer(&request, $f(&request, &params, &instance).await) })
        }
    };
}
pub(crate) use with_params;

plain!(h_create, create);
with_params!(h_open, open);
with_params!(h_rename, rename);
with_params!(h_update, update);
with_params!(h_presence, presence);
with_params!(h_versions, versions);
with_params!(h_add_version, add_version);
with_params!(h_changes, changes);
with_params!(h_checkpoint, checkpoint);

pub(crate) fn object() -> Value {
    json!({"type": "object"})
}

pub fn routes() -> Vec<Route> {
    let route = |method, path, id, summary, handler| Route {
        method,
        path,
        id,
        tag: "docs",
        summary,
        access: Access::Signed,
        response: object,
        handler,
    };
    vec![
        route(Method::Post, "/docs/documents", "docsCreate", "A new document in the drive: {parent, name}.", h_create as ortus_core::api::Handler),
        route(Method::Get, "/docs/documents/:doc", "docsOpen", "A document: its name, label, one's role, its last checkpoint and the updates after it (Yjs, base64), and its sequence number.", h_open),
        route(Method::Patch, "/docs/documents/:doc", "docsRename", "Rename a document: {name}.", h_rename),
        route(Method::Post, "/docs/documents/:doc/updates", "docsUpdate", "An editor's change: the body is a Yjs update; ?client= names the editor.", h_update),
        route(Method::Get, "/docs/documents/:doc/events", "docsEvents", "Changes as they happen (server-sent events): update after ?since=, and presence.", h_events),
        route(Method::Post, "/docs/documents/:doc/presence", "docsPresence", "One's editor is here: {client, state} (a Yjs awareness update, base64).", h_presence),
        route(Method::Get, "/docs/documents/:doc/versions", "docsVersions", "A document's named versions.", h_versions),
        route(Method::Post, "/docs/documents/:doc/versions", "docsAddVersion", "Name the document as it is now: {name}.", h_add_version),
        route(Method::Post, "/docs/documents/:doc/checkpoint", "docsCheckpoint", "An editor's checkpoint: {seq, state (the merged Yjs update, base64), doc (ProseMirror JSON)}. Updates up to seq are folded in.", h_checkpoint),
        route(Method::Get, "/docs/documents/:doc/changes", "docsChanges", "The text now, and as it was in version ?since=.", h_changes),
    ]
}
