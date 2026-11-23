//! Comments on a document, and the review route: a document sent for
//! approval goes through people in order; each approves or sends it back
//! with a comment.

use std::sync::Arc;

use ortus_core::api::{answer, body, ok, ApiError, Method, Route};
use ortus_core::auth::Access;
use ortus_core::instance::Instance;
use ortus_core::storage::BoxFuture;
use rust_web_server::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{document, object, signed, with_params};
use crate::store::{self, now};

fn id(prefix: &str) -> String {
    format!("{prefix}{}", ortus_core::auth::random_hex(12))
}

fn clean(text: &str, max: usize) -> String {
    text.trim()
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect()
}

async fn user_by_email(
    instance: &Instance,
    email: &str,
) -> Result<ortus_core::auth::accounts::User, ApiError> {
    ortus_core::auth::accounts::by_email(
        instance.store.as_ref(),
        &email.trim().to_ascii_lowercase(),
    )
    .await?
    .filter(|u| u.state != ortus_core::auth::accounts::State::Unverified)
    .ok_or_else(|| ApiError::bad_request(format!("{email} is not a colleague.")))
}

async fn name_of(instance: &Instance, user: &str) -> String {
    ortus_core::auth::accounts::by_id(instance.store.as_ref(), user)
        .await
        .ok()
        .flatten()
        .map(|u| u.name)
        .unwrap_or_default()
}

// --- Comments ---

#[derive(Deserialize)]
struct NewComment {
    /// The thread it answers; none starts one (its id is the mark's).
    thread: Option<String>,
    body: String,
    #[serde(default)]
    quote: String,
    /// Someone it is assigned to, by address.
    assign: Option<String>,
}

async fn add_comment(
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
    let input: NewComment = body(request)?;
    let text = clean(&input.body, 5000);
    if text.is_empty() {
        return Err(ApiError::bad_request("A comment says something."));
    }
    // Assigned only to someone who can open the document.
    let assigned = match input.assign.as_deref().filter(|a| !a.trim().is_empty()) {
        Some(email) => {
            let refuse = || {
                ApiError::bad_request(format!(
                    "{} cannot open this document; share it first.",
                    email.trim()
                ))
            };
            let user = user_by_email(instance, email).await.map_err(|_| refuse())?;
            document(instance, &user.id, &item.id, "view")
                .await
                .map_err(|_| refuse())?;
            Some(user.id)
        }
        None => None,
    };
    let comment = id("c");
    let thread = input
        .thread
        .filter(|t| t.len() <= 40 && t.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| comment.clone());
    instance
        .store
        .execute(
            "INSERT INTO docs_comments (id, doc_id, thread_id, author, body, quote, assigned_to, resolved, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8)",
            &[comment.as_str().into(), item.id.as_str().into(), thread.as_str().into(), me.id.as_str().into(), text.as_str().into(), clean(&input.quote, 300).into(), assigned.into(), now().into()],
        )
        .await?;
    list_comments(instance, &item.id)
        .await
        .map(|v| ok(&json!({ "thread": thread, "comments": v })))
}

async fn list_comments(instance: &Instance, doc: &str) -> Result<Value, ApiError> {
    let rows = instance.store.query("SELECT id, thread_id, author, body, quote, assigned_to, resolved, created_at FROM docs_comments WHERE doc_id = $1 ORDER BY created_at, id", &[doc.into()]).await?;
    let mut out = Vec::new();
    for r in rows {
        let author = r.text(2).unwrap_or_default().to_string();
        let assigned = r.text(5).map(str::to_string);
        out.push(json!({
            "id": r.text(0), "thread": r.text(1), "author": name_of(instance, &author).await, "body": r.text(3), "quote": r.text(4),
            "assignedTo": match &assigned { Some(a) => Some(name_of(instance, a).await), None => None },
            "resolved": r.int(6) == Some(1), "at": r.int(7),
        }));
    }
    Ok(json!(out))
}

async fn comments(
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
    Ok(ok(
        &json!({ "comments": list_comments(instance, &item.id).await? }),
    ))
}

async fn resolve(
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
    let thread = params.get("thread").unwrap_or_default();
    instance
        .store
        .execute(
            "UPDATE docs_comments SET resolved = 1 WHERE doc_id = $1 AND thread_id = $2",
            &[item.id.as_str().into(), thread.into()],
        )
        .await?;
    Ok(ok(
        &json!({ "comments": list_comments(instance, &item.id).await? }),
    ))
}

// --- The review route ---

/// The reviews of a document, newest first, with their steps.
pub async fn of_document(instance: &Instance, doc: &str) -> Result<Vec<Value>, ApiError> {
    let rows = instance
        .store
        .query(
            "SELECT id FROM docs_reviews WHERE doc_id = $1 ORDER BY created_at DESC LIMIT 10",
            &[doc.into()],
        )
        .await?;
    let mut out = Vec::new();
    for r in rows {
        if let Some(v) = review(instance, r.text(0).unwrap_or_default()).await? {
            out.push(v);
        }
    }
    Ok(out)
}

async fn review(instance: &Instance, id: &str) -> Result<Option<Value>, ApiError> {
    let s = instance.store.as_ref();
    let rows = s.query("SELECT id, doc_id, created_by, state, version, note, created_at, case_ref, pdf_key, record_path FROM docs_reviews WHERE id = $1", &[id.into()]).await?;
    let Some(r) = rows.first() else {
        return Ok(None);
    };
    let by = r.text(2).unwrap_or_default().to_string();
    let mut steps = Vec::new();
    for st in s.query("SELECT n, user_id, title, due_at, state, comment, decided_at, kind FROM docs_review_steps WHERE review_id = $1 ORDER BY n", &[id.into()]).await? {
        let user = st.text(1).unwrap_or_default().to_string();
        steps.push(json!({"n": st.int(0), "user": user, "name": name_of(instance, &user).await, "title": st.text(2), "dueAt": st.int(3), "state": st.text(4), "comment": st.text(5), "decidedAt": st.int(6), "kind": st.text(7).unwrap_or("approve")}));
    }
    // The version the previous review of this document saw, for "changed since".
    let previous = s
        .query("SELECT version FROM docs_reviews WHERE doc_id = $1 AND created_at < $2 ORDER BY created_at DESC LIMIT 1", &[r.text(1).unwrap_or_default().into(), r.int(6).unwrap_or(0).into()])
        .await?
        .first()
        .and_then(|p| p.int(0));
    Ok(Some(json!({
        "id": r.text(0), "doc": r.text(1), "by": name_of(instance, &by).await, "byId": by, "state": r.text(3), "version": r.int(4),
        "previousVersion": previous, "note": r.text(5), "at": r.int(6), "steps": steps,
        "case": r.text(7), "signed": r.text(8).is_some(), "record": r.text(9).filter(|p| *p != "filing"),
    })))
}

#[derive(Deserialize)]
struct Step {
    email: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "dueAt")]
    due_at: Option<i64>,
    /// "approve" (the default) or "sign".
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Send {
    steps: Vec<Step>,
    #[serde(default)]
    note: String,
    /// The case the signed PDF is filed to ("2026/EM/0114").
    case: Option<String>,
    /// A chat channel told when it is filed ("flood-response").
    channel: Option<String>,
}

/// Sends a document for approval: a version is named, and the first
/// person in the route has their turn.
async fn send(
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
    let input: Send = body(request)?;
    if input.steps.is_empty() || input.steps.len() > 20 {
        return Err(ApiError::bad_request("A route has one to twenty people."));
    }
    let s = instance.store.as_ref();
    if !s
        .query(
            "SELECT 1 FROM docs_reviews WHERE doc_id = $1 AND state = 'open'",
            &[item.id.as_str().into()],
        )
        .await?
        .is_empty()
    {
        return Err(ApiError::conflict(
            "This document is already out for approval.",
        ));
    }
    let mut people = Vec::new();
    for step in &input.steps {
        // Unknown addresses and people without access answer alike.
        let user = user_by_email(instance, &step.email).await.map_err(|_| {
            ApiError::bad_request(format!(
                "{} cannot open this document; share it first.",
                step.email.trim()
            ))
        })?;
        // A reviewer must be able to read it.
        if document(instance, &user.id, &item.id, "view")
            .await
            .is_err()
        {
            return Err(ApiError::bad_request(format!(
                "{} cannot open this document; share it first.",
                step.email.trim()
            )));
        }
        people.push(user.id);
    }
    let kinds: Vec<&str> = input
        .steps
        .iter()
        .map(|st| st.kind.as_deref().unwrap_or("approve"))
        .collect();
    if kinds.iter().any(|k| !matches!(*k, "approve" | "sign")) {
        return Err(ApiError::bad_request("A step approves or signs."));
    }
    let signs = kinds.contains(&"sign");
    // What is approved or signed is the text as it reads: no suggestion
    // may be left open (they read differently before and after).
    let (_, structure) = store::text(s, &item.id).await?;
    if structure.contains("\"type\":\"insertion\"") || structure.contains("\"type\":\"deletion\"") {
        return Err(ApiError::conflict(
            "Accept or reject the open suggestions before sending for approval.",
        ));
    }
    let case = input
        .case
        .as_deref()
        .map(|c| clean(c, 40))
        .filter(|c| !c.is_empty());
    if signs && signing(instance).await.is_none() {
        return Err(ApiError::not_installed("approvals"));
    }
    let version = store::add_version(s, &item.id, &me.id, "Sent for approval").await?;
    let review = id("r");
    // A route that signs or files works on a PDF of the version sent,
    // made now: what is signed is what was sent.
    let pdf_key = if signs || case.is_some() {
        let (_, structure) = store::version(s, &item.id, version)
            .await?
            .unwrap_or_default();
        let doc: Value = serde_json::from_str(&structure).unwrap_or(Value::Null);
        let mut who = Vec::new();
        for (step, user) in input.steps.iter().zip(&people) {
            if step.kind.as_deref() == Some("sign") {
                who.push(crate::pdf::Signatory {
                    name: name_of(instance, user).await,
                    role: clean(&step.title, 100),
                });
            }
        }
        let bytes = crate::pdf::render(&item.name, &doc, item.label.as_deref(), &who, "Signatures");
        let key = format!("docs/reviews/{review}.pdf");
        instance
            .files
            .put(&key, &bytes)
            .await
            .map_err(|_| ApiError::internal())?;
        Some(key)
    } else {
        None
    };
    let mut tx = s.begin().await?;
    tx.execute(
        "INSERT INTO docs_reviews (id, doc_id, created_by, state, version, note, created_at, case_ref, pdf_key, channel) VALUES ($1, $2, $3, 'open', $4, $5, $6, $7, $8, $9)",
        &[review.as_str().into(), item.id.as_str().into(), me.id.as_str().into(), version.into(), clean(&input.note, 1000).into(), now().into(), case.clone().into(), pdf_key.clone().into(), input.channel.as_deref().map(|c| clean(c.trim_start_matches('#'), 80)).filter(|c| !c.is_empty()).into()],
    )
    .await?;
    for (n, (step, user)) in input.steps.iter().zip(&people).enumerate() {
        tx.execute(
            "INSERT INTO docs_review_steps (review_id, n, user_id, title, due_at, state, comment, kind) VALUES ($1, $2, $3, $4, $5, $6, '', $7)",
            &[review.as_str().into(), (n as i64 + 1).into(), user.as_str().into(), clean(&step.title, 100).into(), step.due_at.into(), if n == 0 { "current" } else { "waiting" }.into(), kinds[n].into()],
        )
        .await?;
    }
    tx.commit().await?;
    tell_current(instance, &review).await;
    Ok(ok(&review_json(instance, &review).await?))
}

/// The person whose turn it is in a route hears of it (an approval in the
/// notification centre, with its due time).
async fn tell_current(instance: &Instance, review: &str) {
    let s = instance.store.as_ref();
    let Ok(rows) = s
        .query("SELECT st.user_id, st.due_at, r.doc_id, r.created_by FROM docs_review_steps st JOIN docs_reviews r ON r.id = st.review_id WHERE st.review_id = $1 AND st.state = 'current' AND r.state = 'open'", &[review.into()])
        .await
    else {
        return;
    };
    for r in rows {
        let (user, due, doc, by) = (
            r.text(0).unwrap_or_default().to_string(),
            r.int(1),
            r.text(2).unwrap_or_default().to_string(),
            r.text(3).unwrap_or_default().to_string(),
        );
        let name = match instance
            .services
            .get::<dyn ortus_core::interfaces::Drive>(&instance.switches)
            .await
        {
            Ok(drive) => drive
                .describe(&by, &doc)
                .await
                .ok()
                .flatten()
                .map(|i| i.name)
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        let mut args = json!({"what": name});
        if let Some(d) = due {
            args["due"] = json!(d);
        }
        let notice = ortus_core::notify::Notice {
            app: "docs",
            kind: ortus_core::notify::Kind::Approval,
            title: &format!("Review: {name}"),
            detail: "",
            path: &format!("/docs?review={review}"),
            key: Some("review"),
            args,
        };
        let _ = ortus_core::notify::add(s, &user, &notice).await;
    }
}

async fn review_json(instance: &Instance, id: &str) -> Result<Value, ApiError> {
    review(instance, id)
        .await?
        .ok_or_else(|| ApiError::not_found("There is no such review."))
}

#[derive(Deserialize)]
struct Decision {
    approve: bool,
    #[serde(default)]
    comment: String,
    /// Confirms who signs when the sign-in is older than ten minutes.
    password: Option<String>,
}

pub(crate) async fn signing(
    instance: &Instance,
) -> Option<Arc<dyn ortus_core::interfaces::Signing>> {
    instance
        .services
        .get::<dyn ortus_core::interfaces::Signing>(&instance.switches)
        .await
        .ok()
}

/// Files a finished route's signed PDF to its case (once).
pub async fn file_record(instance: &Instance, review: &str) -> Result<(), String> {
    let s = instance.store.as_ref();
    let rows = s
        .query("SELECT doc_id, created_by, case_ref, pdf_key, record_path, state, channel FROM docs_reviews WHERE id = $1", &[review.into()])
        .await
        .map_err(|e| e.to_string())?;
    let Some(r) = rows.first() else { return Ok(()) };
    let (Some(key), None, Some("approved")) = (r.text(3), r.text(4), r.text(5)) else {
        return Ok(());
    };
    let Ok(records) = instance
        .services
        .get::<dyn ortus_core::interfaces::Records>(&instance.switches)
        .await
    else {
        return Ok(());
    };
    // Claimed first: a route is filed once, even when the last decision
    // and the hourly sweep come together.
    let claimed = s
        .execute("UPDATE docs_reviews SET record_path = 'filing' WHERE id = $1 AND record_path IS NULL AND state = 'approved'", &[review.into()])
        .await
        .map_err(|e| e.to_string())?;
    if claimed != 1 {
        return Ok(());
    }
    let result = file_claimed(instance, review, records.as_ref(), r, key).await;
    let path = match &result {
        Ok(path) => Some(path.clone()),
        Err(_) => None,
    };
    // Filed: its address; not: released for the next try.
    s.execute(
        "UPDATE docs_reviews SET record_path = $1 WHERE id = $2 AND record_path = 'filing'",
        &[path.into(), review.into()],
    )
    .await
    .map_err(|e| e.to_string())?;
    result.map(|_| ())
}

async fn file_claimed(
    instance: &Instance,
    review: &str,
    records: &dyn ortus_core::interfaces::Records,
    r: &ortus_core::storage::Row,
    key: &str,
) -> Result<String, String> {
    let s = instance.store.as_ref();
    let (doc, by) = (
        r.text(0).unwrap_or_default().to_string(),
        r.text(1).unwrap_or_default().to_string(),
    );
    let pdf = instance
        .files
        .get(key)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the route's PDF is missing")?;
    let mut readers: Vec<String> = s
        .query(
            "SELECT user_id FROM docs_review_steps WHERE review_id = $1",
            &[review.into()],
        )
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .filter_map(|x| x.text(0).map(str::to_string))
        .collect();
    readers.push(by.clone());
    let drive = crate::api::drive(instance).await.map_err(|e| e.message)?;
    let item = drive
        .describe(&by, &doc)
        .await?
        .ok_or("the document is gone")?;
    let filed = records
        .file(ortus_core::interfaces::NewRecord {
            by: &by,
            title: &item.name,
            kind: "decision",
            case: r.text(2),
            label: item.label.as_deref(),
            pdf: &pdf,
            source: &format!("/docs?doc={doc}"),
            readers: &readers,
            channel: r.text(6),
        })
        .await?;
    Ok(filed.path)
}

/// The person whose turn it is approves, or sends the document back.
async fn decide(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: Decision = body(request)?;
    let id = params.get("review").unwrap_or_default();
    let s = instance.store.as_ref();
    let rows = s
        .query(
            "SELECT doc_id, state FROM docs_reviews WHERE id = $1",
            &[id.into()],
        )
        .await?;
    let r = rows
        .first()
        .ok_or_else(|| ApiError::not_found("There is no such review."))?;
    let doc = r.text(0).unwrap_or_default().to_string();
    document(instance, &me.id, &doc, "view").await?;
    if r.text(1) != Some("open") {
        return Err(ApiError::conflict("This review is over."));
    }
    let comment = clean(&input.comment, 2000);
    if !input.approve && comment.is_empty() {
        return Err(ApiError::bad_request("Say why it goes back."));
    }
    // A signing step: the person confirms who they are (a recent sign-in
    // or their password), then signs the route's PDF with their own key.
    let step = s
        .query("SELECT n, kind, title FROM docs_review_steps WHERE review_id = $1 AND state = 'current' AND user_id = $2", &[id.into(), me.id.as_str().into()])
        .await?;
    let signed_pdf = match step.first() {
        Some(st) if input.approve && st.text(1) == Some("sign") => {
            let session = ortus_core::auth::current(request, instance)
                .await
                .ok_or_else(ApiError::sign_in)?;
            // A recent strong sign-in, or a confirmation (step-up), counts:
            // not a session made by a phone link or a recovery code.
            let fresh = ortus_core::auth::step_up::confirmed(s, &session).await?;
            // No password given: the client confirms (a passkey, or the
            // password and code) through the shell's step-up and tries again.
            if !fresh && input.password.as_deref().is_none_or(str::is_empty) {
                ortus_core::auth::step_up::require(s, &session).await?;
            }
            let confirmed = match input.password.as_deref().filter(|p| !p.is_empty()) {
                Some(p) => {
                    // As slow to guess as signing in.
                    let key = format!("sign-confirm:{}", me.id);
                    if let Some(until) =
                        ortus_core::auth::limits::blocked_until(s, &key, now()).await?
                    {
                        return Err(ApiError::too_many(format!(
                            "Too many tries. Try again in {} seconds.",
                            (until - now()).max(1000) / 1000
                        )));
                    }
                    let good =
                        ortus_core::auth::credentials::check_password(s, &me.id, p, now()).await?;
                    if good {
                        ortus_core::auth::limits::succeeded(s, &[key]).await?;
                    }
                    good
                }
                None => false,
            };
            if !fresh && !confirmed {
                return Err(ApiError {
                    status: 403,
                    code: "confirm",
                    message: if input.password.is_some() {
                        "That password is not right.".into()
                    } else {
                        "Confirm it is you before signing.".into()
                    },
                });
            }
            let key = s
                .query(
                    "SELECT pdf_key FROM docs_reviews WHERE id = $1",
                    &[id.into()],
                )
                .await?
                .first()
                .and_then(|r| r.text(0).map(str::to_string))
                .ok_or_else(|| ApiError::conflict("This route has no PDF to sign."))?;
            let pdf = instance
                .files
                .get(&key)
                .await
                .map_err(|_| ApiError::internal())?
                .ok_or_else(ApiError::internal)?;
            let signer = signing(instance)
                .await
                .ok_or_else(|| ApiError::not_installed("approvals"))?;
            let reason = st
                .text(2)
                .filter(|t| !t.is_empty())
                .unwrap_or("Signed")
                .to_string();
            let signed = signer
                .sign(&me.id, &pdf, &reason)
                .await
                .map_err(|_| ApiError::internal())?;
            let _ = key;
            // Each signature in a file of its own: the route points at it
            // only when the step is decided.
            let n = st.int(0).unwrap_or(0);
            Some((format!("docs/reviews/{id}-{n}.pdf"), signed))
        }
        _ => None,
    };
    let mut tx = s.begin().await?;
    let current = tx.query("SELECT n FROM docs_review_steps WHERE review_id = $1 AND state = 'current' AND user_id = $2", &[id.into(), me.id.as_str().into()]).await?;
    let Some(n) = current.first().and_then(|c| c.int(0)) else {
        tx.rollback().await?;
        return Err(ApiError::forbidden());
    };
    // Only a step still waiting for this person is decided: a second
    // click (or a second tab) finds it decided already.
    let decided = tx.execute(
        "UPDATE docs_review_steps SET state = $1, comment = $2, decided_at = $3 WHERE review_id = $4 AND n = $5 AND user_id = $6 AND state = 'current'",
        &[if input.approve { "approved" } else { "returned" }.into(), comment.as_str().into(), now().into(), id.into(), n.into(), me.id.as_str().into()],
    )
    .await?;
    if decided != 1 {
        tx.rollback().await?;
        return Err(ApiError::conflict("This step is already decided."));
    }
    if let Some((key, bytes)) = &signed_pdf {
        instance
            .files
            .put(key, bytes)
            .await
            .map_err(|_| ApiError::internal())?;
        tx.execute(
            "UPDATE docs_review_steps SET state = 'signed' WHERE review_id = $1 AND n = $2",
            &[id.into(), n.into()],
        )
        .await?;
        tx.execute(
            "UPDATE docs_reviews SET pdf_key = $1 WHERE id = $2",
            &[key.as_str().into(), id.into()],
        )
        .await?;
    }
    let approved = input.approve;
    if input.approve {
        store::advance(tx.as_mut(), id).await?;
    } else {
        tx.execute(
            "UPDATE docs_reviews SET state = 'returned' WHERE id = $1",
            &[id.into()],
        )
        .await?;
        tx.execute("UPDATE docs_review_steps SET state = 'cancelled' WHERE review_id = $1 AND state = 'waiting'", &[id.into()]).await?;
    }
    tx.commit().await?;
    if approved {
        tell_current(instance, id).await;
    }
    // The route is over: its signed PDF goes to the case (the hourly
    // sweep tries again if the records app could not take it now).
    let _ = file_record(instance, id).await;
    ortus_core::auth::audit(
        instance,
        ortus_core::audit::Event::new(
            if signed_pdf.is_some() {
                "docs.signed"
            } else if input.approve {
                "docs.approved"
            } else {
                "docs.returned"
            },
            ortus_core::audit::Outcome::Done,
        )
        .by(&me.id)
        .on(&doc)
        .with(json!({"review": id, "step": n})),
    )
    .await;
    // The whole route approved: other systems hear of it (approval.done).
    let finished = s
        .query("SELECT state FROM docs_reviews WHERE id = $1", &[id.into()])
        .await?
        .first()
        .is_some_and(|r| r.text(0) == Some("approved"));
    if approved && finished {
        ortus_core::auth::audit(
            instance,
            ortus_core::audit::Event::new("docs.review_done", ortus_core::audit::Outcome::Done)
                .by(&me.id)
                .on(&doc)
                .with(json!({"review": id})),
        )
        .await;
    }
    Ok(ok(&review_json(instance, id).await?))
}

async fn get_review(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let v = review_json(instance, params.get("review").unwrap_or_default()).await?;
    document(
        instance,
        &me.id,
        v["doc"].as_str().unwrap_or_default(),
        "view",
    )
    .await?;
    Ok(ok(&v))
}

/// An editor of the document withdraws a route that is still open.
async fn cancel(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let id = params.get("review").unwrap_or_default();
    let v = review_json(instance, id).await?;
    let doc = v["doc"].as_str().unwrap_or_default().to_string();
    document(instance, &me.id, &doc, "edit").await?;
    let s = instance.store.as_ref();
    let mut tx = s.begin().await?;
    let changed = tx
        .execute(
            "UPDATE docs_reviews SET state = 'cancelled' WHERE id = $1 AND state = 'open'",
            &[id.into()],
        )
        .await?;
    if changed != 1 {
        tx.rollback().await?;
        return Err(ApiError::conflict("This review is over."));
    }
    tx.execute("UPDATE docs_review_steps SET state = 'cancelled' WHERE review_id = $1 AND state IN ('waiting', 'current')", &[id.into()]).await?;
    tx.commit().await?;
    ortus_core::auth::audit(
        instance,
        ortus_core::audit::Event::new("docs.review_cancelled", ortus_core::audit::Outcome::Done)
            .by(&me.id)
            .on(&doc)
            .with(json!({"review": id})),
    )
    .await;
    Ok(ok(&review_json(instance, id).await?))
}

/// What a review is about: the document as it was sent (the named
/// version), the text of the version the review before saw, and whether
/// it was edited since it was sent.
async fn review_document(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let v = review_json(instance, params.get("review").unwrap_or_default()).await?;
    let item = document(
        instance,
        &me.id,
        v["doc"].as_str().unwrap_or_default(),
        "view",
    )
    .await?;
    let s = instance.store.as_ref();
    let (text, structure) = store::version(s, &item.id, v["version"].as_i64().unwrap_or(0))
        .await?
        .unwrap_or_default();
    let before = match v["previousVersion"].as_i64() {
        Some(n) => store::version(s, &item.id, n).await?.map(|(t, _)| t),
        None => None,
    };
    let (now_text, _) = store::text(s, &item.id).await?;
    let doc: Value = serde_json::from_str(&structure).unwrap_or(Value::Null);
    Ok(ok(&json!({
        "name": item.name, "label": item.label, "doc": doc,
        "before": before, "edited": now_text != text,
    })))
}

/// The route's PDF with the signatures made so far.
async fn review_pdf(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let id = params.get("review").unwrap_or_default();
    let v = review_json(instance, id).await?;
    let item = document(
        instance,
        &me.id,
        v["doc"].as_str().unwrap_or_default(),
        "view",
    )
    .await?;
    let key = instance
        .store
        .query(
            "SELECT pdf_key FROM docs_reviews WHERE id = $1",
            &[id.into()],
        )
        .await?
        .first()
        .and_then(|r| r.text(0).map(str::to_string))
        .ok_or_else(|| ApiError::not_found("This route has no PDF."))?;
    let bytes = instance
        .files
        .get(&key)
        .await
        .map_err(|_| ApiError::internal())?
        .ok_or_else(|| ApiError::not_found("This route has no PDF."))?;
    let name: String = item
        .name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " ._-".contains(*c))
        .collect();
    let header = |n: &'static str, v: String| rust_web_server::header::Header {
        name: n.into(),
        value: v.into(),
    };
    let headers = vec![
        header(
            "Content-Disposition",
            format!("inline; filename=\"{}.pdf\"", name.trim()),
        ),
        header("X-Content-Type-Options", "nosniff".into()),
        header(
            "Content-Security-Policy",
            "sandbox; default-src 'none'".into(),
        ),
        header("Cache-Control", "private, no-store".into()),
    ];
    let body =
        rust_web_server::range::Range::get_content_range(bytes, "application/pdf".to_string());
    Ok(Response::get_response(
        STATUS_CODE_REASON_PHRASE.n200_ok,
        Some(headers),
        Some(vec![body]),
    ))
}

/// Reviews where it is the person's turn, and their own documents out for approval.
async fn mine(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let s = instance.store.as_ref();
    let mut waiting = Vec::new();
    for r in s.query("SELECT r.id FROM docs_reviews r JOIN docs_review_steps st ON st.review_id = r.id WHERE st.user_id = $1 AND st.state = 'current' AND r.state = 'open' ORDER BY r.created_at DESC LIMIT 50", &[me.id.as_str().into()]).await? {
        if let Some(mut v) = review(instance, r.text(0).unwrap_or_default()).await? {
            if let Ok(item) = document(instance, &me.id, v["doc"].as_str().unwrap_or_default(), "view").await {
                v["name"] = json!(item.name);
                waiting.push(v);
            }
        }
    }
    Ok(ok(&json!({ "waiting": waiting })))
}

with_params!(h_add_comment, add_comment);
with_params!(h_comments, comments);
with_params!(h_resolve, resolve);
with_params!(h_send, send);
with_params!(h_decide, decide);
with_params!(h_get_review, get_review);
with_params!(h_cancel, cancel);
with_params!(h_review_document, review_document);
with_params!(h_review_pdf, review_pdf);

fn h_mine(
    request: Request,
    _: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move { answer(&request, mine(&request, &instance).await) })
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
        route(Method::Get, "/docs/documents/:doc/comments", "docsComments", "A document's comments, by thread.", h_comments as ortus_core::api::Handler),
        route(Method::Post, "/docs/documents/:doc/comments", "docsComment", "Comment: {thread (to answer one), body, quote, assign (an address)}.", h_add_comment),
        route(Method::Post, "/docs/documents/:doc/comments/:thread/resolve", "docsResolve", "Mark a thread resolved.", h_resolve),
        route(Method::Post, "/docs/documents/:doc/reviews", "docsSendForApproval", "Send for approval: {steps: [{email, title, dueAt, kind: approve|sign}], note, case}. A version is named; a route that signs works on a PDF of it, filed to the case when everyone has signed.", h_send),
        route(Method::Get, "/docs/reviews/mine", "docsMyReviews", "Reviews where it is one's turn.", h_mine),
        route(Method::Get, "/docs/reviews/:review", "docsReview", "A review with its steps.", h_get_review),
        route(Method::Post, "/docs/reviews/:review/decide", "docsDecide", "The person whose turn it is: {approve, comment, password}. Sending back needs a comment; signing needs a sign-in in the last ten minutes or the password.", h_decide),
        route(Method::Post, "/docs/reviews/:review/cancel", "docsReviewCancel", "An editor withdraws a route still open.", h_cancel),
        route(Method::Get, "/docs/reviews/:review/document", "docsReviewDocument", "The document as it was sent, the text the review before saw, and whether it was edited since.", h_review_document),
        route(Method::Get, "/docs/reviews/:review/pdf", "docsReviewPdf", "The PDF a route signs, with the signatures made so far.", h_review_pdf),
    ]
}
