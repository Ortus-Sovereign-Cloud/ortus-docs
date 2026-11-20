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
    let assigned = match input.assign.as_deref().filter(|a| !a.trim().is_empty()) {
        Some(email) => Some(user_by_email(instance, email).await?.id),
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
    let rows = s.query("SELECT id, doc_id, created_by, state, version, note, created_at FROM docs_reviews WHERE id = $1", &[id.into()]).await?;
    let Some(r) = rows.first() else {
        return Ok(None);
    };
    let by = r.text(2).unwrap_or_default().to_string();
    let mut steps = Vec::new();
    for st in s.query("SELECT n, user_id, title, due_at, state, comment, decided_at FROM docs_review_steps WHERE review_id = $1 ORDER BY n", &[id.into()]).await? {
        let user = st.text(1).unwrap_or_default().to_string();
        steps.push(json!({"n": st.int(0), "user": user, "name": name_of(instance, &user).await, "title": st.text(2), "dueAt": st.int(3), "state": st.text(4), "comment": st.text(5), "decidedAt": st.int(6)}));
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
    })))
}

#[derive(Deserialize)]
struct Step {
    email: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "dueAt")]
    due_at: Option<i64>,
}

#[derive(Deserialize)]
struct Send {
    steps: Vec<Step>,
    #[serde(default)]
    note: String,
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
        let user = user_by_email(instance, &step.email).await?;
        // A reviewer must be able to read it.
        if document(instance, &user.id, &item.id, "view")
            .await
            .is_err()
        {
            return Err(ApiError::bad_request(format!(
                "{} cannot open this document; share it first.",
                user.name
            )));
        }
        people.push(user.id);
    }
    let version = store::add_version(s, &item.id, &me.id, "Sent for approval").await?;
    let review = id("r");
    let mut tx = s.begin().await?;
    tx.execute(
        "INSERT INTO docs_reviews (id, doc_id, created_by, state, version, note, created_at) VALUES ($1, $2, $3, 'open', $4, $5, $6)",
        &[review.as_str().into(), item.id.as_str().into(), me.id.as_str().into(), version.into(), clean(&input.note, 1000).into(), now().into()],
    )
    .await?;
    for (n, (step, user)) in input.steps.iter().zip(&people).enumerate() {
        tx.execute(
            "INSERT INTO docs_review_steps (review_id, n, user_id, title, due_at, state, comment) VALUES ($1, $2, $3, $4, $5, $6, '')",
            &[review.as_str().into(), (n as i64 + 1).into(), user.as_str().into(), clean(&step.title, 100).into(), step.due_at.into(), if n == 0 { "current" } else { "waiting" }.into()],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(ok(&review_json(instance, &review).await?))
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
    let mut tx = s.begin().await?;
    let current = tx.query("SELECT n FROM docs_review_steps WHERE review_id = $1 AND state = 'current' AND user_id = $2", &[id.into(), me.id.as_str().into()]).await?;
    let Some(n) = current.first().and_then(|c| c.int(0)) else {
        tx.rollback().await?;
        return Err(ApiError::forbidden());
    };
    tx.execute(
        "UPDATE docs_review_steps SET state = $1, comment = $2, decided_at = $3 WHERE review_id = $4 AND n = $5",
        &[if input.approve { "approved" } else { "returned" }.into(), comment.as_str().into(), now().into(), id.into(), n.into()],
    )
    .await?;
    if input.approve {
        let next = tx.execute("UPDATE docs_review_steps SET state = 'current' WHERE review_id = $1 AND n = $2 AND state = 'waiting'", &[id.into(), (n + 1).into()]).await?;
        if next == 0 {
            tx.execute(
                "UPDATE docs_reviews SET state = 'approved' WHERE id = $1",
                &[id.into()],
            )
            .await?;
        }
    } else {
        tx.execute(
            "UPDATE docs_reviews SET state = 'returned' WHERE id = $1",
            &[id.into()],
        )
        .await?;
        tx.execute("UPDATE docs_review_steps SET state = 'cancelled' WHERE review_id = $1 AND state = 'waiting'", &[id.into()]).await?;
    }
    tx.commit().await?;
    ortus_core::auth::audit(
        instance,
        ortus_core::audit::Event::new(
            if input.approve {
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
        route(Method::Post, "/docs/documents/:doc/reviews", "docsSendForApproval", "Send for approval: {steps: [{email, title, dueAt}], note}. A version is named; the first person has their turn.", h_send),
        route(Method::Get, "/docs/reviews/mine", "docsMyReviews", "Reviews where it is one's turn.", h_mine),
        route(Method::Get, "/docs/reviews/:review", "docsReview", "A review with its steps.", h_get_review),
        route(Method::Post, "/docs/reviews/:review/decide", "docsDecide", "The person whose turn it is: {approve, comment}. Sending back needs a comment.", h_decide),
    ]
}
