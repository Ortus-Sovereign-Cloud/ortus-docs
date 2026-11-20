//! The docs store: each document is a Yjs document kept as its updates
//! (in order) and a merged snapshot; who is in it (presence); comments;
//! named versions; reviews. The document itself (its name, folder, label
//! and who may open it) is an item of the drive.

use ortus_core::storage::{AsyncStore, AsyncTx, Migration, StoreResult, Value};

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "documents, presence, comments, versions, reviews",
        sql: "CREATE TABLE docs_updates (
        doc_id TEXT NOT NULL,
        seq BIGINT NOT NULL,
        data {{blob}} NOT NULL,
        client TEXT NOT NULL,
        user_id TEXT NOT NULL,
        at BIGINT NOT NULL,
        PRIMARY KEY (doc_id, seq)
    );
    CREATE TABLE docs_snapshots (
        doc_id TEXT PRIMARY KEY,
        data {{blob}} NOT NULL,
        seq BIGINT NOT NULL,
        text TEXT NOT NULL,
        json TEXT NOT NULL,
        updated_at BIGINT NOT NULL
    );
    CREATE TABLE docs_presence (
        doc_id TEXT NOT NULL,
        client TEXT NOT NULL,
        user_id TEXT NOT NULL,
        state TEXT NOT NULL,
        at BIGINT NOT NULL,
        PRIMARY KEY (doc_id, client)
    );
    CREATE TABLE docs_comments (
        id TEXT PRIMARY KEY,
        doc_id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        author TEXT NOT NULL,
        body TEXT NOT NULL,
        quote TEXT NOT NULL,
        assigned_to TEXT,
        resolved BIGINT NOT NULL,
        created_at BIGINT NOT NULL
    );
    CREATE INDEX docs_comments_doc ON docs_comments (doc_id, created_at);
    CREATE TABLE docs_versions (
        doc_id TEXT NOT NULL,
        n BIGINT NOT NULL,
        name TEXT NOT NULL,
        data {{blob}} NOT NULL,
        text TEXT NOT NULL,
        created_by TEXT NOT NULL,
        created_at BIGINT NOT NULL,
        PRIMARY KEY (doc_id, n)
    );
    CREATE TABLE docs_reviews (
        id TEXT PRIMARY KEY,
        doc_id TEXT NOT NULL,
        created_by TEXT NOT NULL,
        state TEXT NOT NULL,
        version BIGINT NOT NULL,
        note TEXT NOT NULL,
        created_at BIGINT NOT NULL
    );
    CREATE INDEX docs_reviews_doc ON docs_reviews (doc_id, created_at);
    CREATE TABLE docs_review_steps (
        review_id TEXT NOT NULL,
        n BIGINT NOT NULL,
        user_id TEXT NOT NULL,
        title TEXT NOT NULL,
        due_at BIGINT,
        state TEXT NOT NULL,
        comment TEXT NOT NULL,
        decided_at BIGINT,
        PRIMARY KEY (review_id, n)
    );
    CREATE INDEX docs_review_steps_user ON docs_review_steps (user_id, state)",
    },
    Migration {
        version: 2,
        name: "document owners, versions' structure",
        sql: "CREATE TABLE docs_documents (
        doc_id TEXT PRIMARY KEY,
        owner_id TEXT NOT NULL,
        created_at BIGINT NOT NULL
    );
    CREATE INDEX docs_documents_owner ON docs_documents (owner_id);
    ALTER TABLE docs_versions ADD COLUMN json TEXT",
    },
    Migration {
        version: 3,
        name: "routes that sign and file",
        sql: "ALTER TABLE docs_review_steps ADD COLUMN kind TEXT NOT NULL DEFAULT 'approve';
    ALTER TABLE docs_reviews ADD COLUMN case_ref TEXT;
    ALTER TABLE docs_reviews ADD COLUMN pdf_key TEXT;
    ALTER TABLE docs_reviews ADD COLUMN record_path TEXT",
    },
    Migration {
        version: 4,
        name: "the channel a filed route tells",
        sql: "ALTER TABLE docs_reviews ADD COLUMN channel TEXT",
    },
    Migration {
        version: 5,
        name: "the drive owner of each document",
        sql: "ALTER TABLE docs_documents ADD COLUMN drive_owner TEXT",
    },
];

/// An update larger than this is refused.
pub const MAX_UPDATE: usize = 2 * 1024 * 1024;
/// Loose updates of one document at most (and their bytes), before an
/// editor's checkpoint.
pub const MAX_LOOSE: i64 = 5000;
pub const MAX_LOOSE_BYTES: i64 = 64 * 1024 * 1024;
/// Editors of one person in one document at once (tabs, devices).
pub const MAX_CLIENTS: i64 = 8;

pub fn now() -> i64 {
    ortus_core::program::now_ms()
}

/// Whether bytes may be a Yjs update (v1): not empty, not too big. The
/// browsers that apply it check the rest.
pub fn valid_update(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.len() <= MAX_UPDATE
}

/// Text of the editor's structure (ProseMirror JSON): blocks on their
/// own lines, cells apart.
pub fn plain(doc: &serde_json::Value) -> String {
    fn walk(node: &serde_json::Value, out: &mut String, depth: usize) {
        if depth > 64 || out.len() > 1_000_000 {
            return;
        }
        if let Some(text) = node["text"].as_str() {
            out.push_str(text);
        }
        for child in node["content"].as_array().into_iter().flatten() {
            walk(child, out, depth + 1);
        }
        match node["type"].as_str().unwrap_or_default() {
            "paragraph" | "heading" | "table_row" | "list_item" | "blockquote" => out.push('\n'),
            "table_cell" | "table_header" => out.push('\t'),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(doc, &mut out, 0);
    out.trim().to_string()
}

/// The latest checkpoint (merged state, its sequence number, text and
/// structure) and every update after it, oldest first.
pub async fn state(
    store: &dyn AsyncStore,
    doc: &str,
) -> StoreResult<(Option<Vec<u8>>, i64, Vec<(i64, Vec<u8>)>)> {
    // One read: a checkpoint between two reads would leave a gap.
    let mut tx = store.begin_read().await?;
    let snapshot = tx
        .query(
            "SELECT data, seq FROM docs_snapshots WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?;
    let (base, seq) = match snapshot.first() {
        Some(r) => (
            Some(r.blob(0).unwrap_or_default().to_vec()),
            r.int(1).unwrap_or(0),
        ),
        None => (None, 0),
    };
    let after = tx
        .query(
            "SELECT seq, data FROM docs_updates WHERE doc_id = $1 AND seq > $2 ORDER BY seq",
            &[doc.into(), seq.into()],
        )
        .await?
        .iter()
        .map(|r| {
            (
                r.int(0).unwrap_or(0),
                r.blob(1).unwrap_or_default().to_vec(),
            )
        })
        .collect();
    tx.rollback().await?;
    Ok((base, seq, after))
}

/// The latest checkpoint's text and structure.
pub async fn text(store: &dyn AsyncStore, doc: &str) -> StoreResult<(String, String)> {
    Ok(store
        .query(
            "SELECT text, json FROM docs_snapshots WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .map(|r| {
            (
                r.text(0).unwrap_or_default().to_string(),
                r.text(1).unwrap_or_default().to_string(),
            )
        })
        .unwrap_or_default())
}

/// An editor's checkpoint: the whole document merged up to `seq`, with
/// its text and structure. Updates up to `seq` are folded into it. A
/// checkpoint older than the one kept is ignored.
pub async fn checkpoint(
    store: &dyn AsyncStore,
    doc: &str,
    seq: i64,
    data: &[u8],
    structure: &serde_json::Value,
) -> StoreResult<bool> {
    let text = plain(structure);
    let mut tx = store.begin().await?;
    let last = tx
        .query(
            "SELECT COALESCE(MAX(seq), 0) FROM docs_updates WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0))
        .unwrap_or(0);
    let kept = tx
        .query(
            "SELECT seq FROM docs_snapshots WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0));
    // It must fold in updates that exist, and be newer than the one kept.
    if seq > last.max(kept.unwrap_or(0)) || kept.is_some_and(|k| seq <= k) {
        tx.rollback().await?;
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM docs_snapshots WHERE doc_id = $1",
        &[doc.into()],
    )
    .await?;
    tx.execute(
        "INSERT INTO docs_snapshots (doc_id, data, seq, text, json, updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
        &[doc.into(), Value::Blob(data.to_vec()), seq.into(), text.as_str().into(), structure.to_string().into(), now().into()],
    )
    .await?;
    tx.execute(
        "DELETE FROM docs_updates WHERE doc_id = $1 AND seq <= $2",
        &[doc.into(), seq.into()],
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// The sequence number the kept checkpoint folds in (0: none).
pub async fn kept_seq(store: &dyn AsyncStore, doc: &str) -> StoreResult<i64> {
    Ok(store
        .query(
            "SELECT seq FROM docs_snapshots WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0))
        .unwrap_or(0))
}

/// Loose updates of a document: how many, and their bytes.
pub async fn loose(store: &dyn AsyncStore, doc: &str) -> StoreResult<(i64, i64)> {
    Ok(store
        .query(
            "SELECT CAST(COUNT(*) AS BIGINT), CAST(COALESCE(SUM(LENGTH(data)), 0) AS BIGINT) FROM docs_updates WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .map(|r| (r.int(0).unwrap_or(0), r.int(1).unwrap_or(0)))
        .unwrap_or((0, 0)))
}

/// Adds an update; answers its sequence number.
pub async fn add_update(
    store: &dyn AsyncStore,
    doc: &str,
    user: &str,
    client: &str,
    data: &[u8],
) -> StoreResult<i64> {
    let mut tx = store.begin().await?;
    // One writer of a document at a time picks the next number.
    let last = tx
        .query(
            "SELECT COALESCE(MAX(seq), 0) FROM docs_updates WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0))
        .unwrap_or(0);
    let snap = tx
        .query(
            "SELECT seq FROM docs_snapshots WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0))
        .unwrap_or(0);
    let seq = last.max(snap) + 1;
    tx.execute(
        "INSERT INTO docs_updates (doc_id, seq, data, client, user_id, at) VALUES ($1, $2, $3, $4, $5, $6)",
        &[doc.into(), seq.into(), Value::Blob(data.to_vec()), client.into(), user.into(), now().into()],
    )
    .await?;
    tx.commit().await?;
    Ok(seq)
}

/// Updates after `since`, oldest first.
pub async fn updates_since(
    store: &dyn AsyncStore,
    doc: &str,
    since: i64,
) -> StoreResult<Vec<(i64, Vec<u8>, String)>> {
    Ok(store
        .query("SELECT seq, data, client FROM docs_updates WHERE doc_id = $1 AND seq > $2 ORDER BY seq LIMIT 500", &[doc.into(), since.into()])
        .await?
        .iter()
        .map(|r| (r.int(0).unwrap_or(0), r.blob(1).unwrap_or_default().to_vec(), r.text(2).unwrap_or_default().to_string()))
        .collect())
}

/// Someone is in a document (their editor's awareness state).
pub async fn set_presence(
    store: &dyn AsyncStore,
    doc: &str,
    client: &str,
    user: &str,
    state: &str,
) -> StoreResult<()> {
    let mut tx = store.begin().await?;
    tx.execute(
        "DELETE FROM docs_presence WHERE doc_id = $1 AND client = $2",
        &[doc.into(), client.into()],
    )
    .await?;
    // Old rows go; one person keeps a few editors at most.
    tx.execute(
        "DELETE FROM docs_presence WHERE doc_id = $1 AND at < $2",
        &[doc.into(), (now() - 60_000).into()],
    )
    .await?;
    let theirs: Vec<String> = tx
        .query(
            "SELECT client FROM docs_presence WHERE doc_id = $1 AND user_id = $2 ORDER BY at DESC",
            &[doc.into(), user.into()],
        )
        .await?
        .iter()
        .filter_map(|r| r.text(0).map(str::to_string))
        .collect();
    for old in theirs.iter().skip(MAX_CLIENTS as usize - 1) {
        tx.execute(
            "DELETE FROM docs_presence WHERE doc_id = $1 AND client = $2",
            &[doc.into(), old.as_str().into()],
        )
        .await?;
    }
    tx.execute("INSERT INTO docs_presence (doc_id, client, user_id, state, at) VALUES ($1, $2, $3, $4, $5)", &[doc.into(), client.into(), user.into(), state.into(), now().into()]).await?;
    tx.commit().await?;
    Ok(())
}

/// Who is in a document now (seen in the last 30 seconds).
pub async fn presence(
    store: &dyn AsyncStore,
    doc: &str,
) -> StoreResult<Vec<(String, String, String, i64)>> {
    Ok(store
        .query("SELECT client, user_id, state, at FROM docs_presence WHERE doc_id = $1 AND at > $2 ORDER BY at", &[doc.into(), (now() - 30_000).into()])
        .await?
        .iter()
        .map(|r| (r.text(0).unwrap_or_default().into(), r.text(1).unwrap_or_default().into(), r.text(2).unwrap_or_default().into(), r.int(3).unwrap_or(0)))
        .collect())
}

/// A named version: the latest checkpoint (editors send one first) with
/// the updates after it.
pub async fn add_version(
    store: &dyn AsyncStore,
    doc: &str,
    user: &str,
    name: &str,
) -> StoreResult<i64> {
    let (base, seq, after) = state(store, doc).await?;
    let (text, structure) = text(store, doc).await?;
    let packed = serde_json::json!({"seq": seq, "base": base.map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<String>()), "after": after.iter().map(|(n, d)| (n, d.iter().map(|x| format!("{x:02x}")).collect::<String>())).collect::<Vec<_>>()}).to_string();
    let mut tx = store.begin().await?;
    let n = tx
        .query(
            "SELECT COALESCE(MAX(n), 0) FROM docs_versions WHERE doc_id = $1",
            &[doc.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0))
        .unwrap_or(0)
        + 1;
    tx.execute(
        "INSERT INTO docs_versions (doc_id, n, name, data, text, created_by, created_at, json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        &[doc.into(), n.into(), name.into(), Value::Blob(packed.into_bytes()), text.as_str().into(), user.into(), now().into(), structure.as_str().into()],
    )
    .await?;
    tx.commit().await?;
    Ok(n)
}

/// A version's text.
pub async fn version_text(
    store: &dyn AsyncStore,
    doc: &str,
    n: i64,
) -> StoreResult<Option<String>> {
    Ok(store
        .query(
            "SELECT text FROM docs_versions WHERE doc_id = $1 AND n = $2",
            &[doc.into(), n.into()],
        )
        .await?
        .first()
        .and_then(|r| r.text(0).map(str::to_string)))
}

/// A version's text and structure.
pub async fn version(
    store: &dyn AsyncStore,
    doc: &str,
    n: i64,
) -> StoreResult<Option<(String, String)>> {
    Ok(store
        .query(
            "SELECT text, json FROM docs_versions WHERE doc_id = $1 AND n = $2",
            &[doc.into(), n.into()],
        )
        .await?
        .first()
        .map(|r| {
            (
                r.text(0).unwrap_or_default().to_string(),
                r.text(1).unwrap_or("null").to_string(),
            )
        }))
}

/// Gives the turn to the next person waiting in a route, or closes it
/// as approved when nobody is left.
pub async fn advance(tx: &mut dyn AsyncTx, review: &str) -> StoreResult<()> {
    let next = tx
        .query(
            "SELECT MIN(n) FROM docs_review_steps WHERE review_id = $1 AND state = 'waiting'",
            &[review.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0));
    match next {
        Some(n) => {
            tx.execute(
                "UPDATE docs_review_steps SET state = 'current' WHERE review_id = $1 AND n = $2",
                &[review.into(), n.into()],
            )
            .await?;
        }
        None => {
            tx.execute(
                "UPDATE docs_reviews SET state = 'approved' WHERE id = $1 AND state = 'open'",
                &[review.into()],
            )
            .await?;
        }
    }
    Ok(())
}

/// Everything of a document (it was deleted from the drive for good).
pub async fn forget_document(tx: &mut dyn AsyncTx, doc: &str) -> StoreResult<()> {
    for sql in [
        "DELETE FROM docs_updates WHERE doc_id = $1",
        "DELETE FROM docs_snapshots WHERE doc_id = $1",
        "DELETE FROM docs_presence WHERE doc_id = $1",
        "DELETE FROM docs_comments WHERE doc_id = $1",
        "DELETE FROM docs_versions WHERE doc_id = $1",
        "DELETE FROM docs_review_steps WHERE review_id IN (SELECT id FROM docs_reviews WHERE doc_id = $1)",
        "DELETE FROM docs_reviews WHERE doc_id = $1",
        "DELETE FROM docs_documents WHERE doc_id = $1",
    ] {
        tx.execute(sql, &[doc.into()]).await?;
    }
    Ok(())
}

/// A person's account is gone: their presence, their comments, their
/// name on updates, versions and routes; their turns in routes are
/// skipped (the next person has theirs). Their documents go with their
/// drive (the sweep finds them).
pub async fn forget(tx: &mut dyn AsyncTx, user: &str) -> StoreResult<()> {
    let turns: Vec<String> = tx
        .query(
            "SELECT review_id FROM docs_review_steps WHERE user_id = $1 AND state = 'current'",
            &[user.into()],
        )
        .await?
        .iter()
        .filter_map(|r| r.text(0).map(str::to_string))
        .collect();
    for sql in [
        "DELETE FROM docs_presence WHERE user_id = $1",
        "UPDATE docs_review_steps SET state = 'skipped' WHERE user_id = $1 AND state IN ('waiting', 'current')",
        "DELETE FROM docs_comments WHERE author = $1",
        "UPDATE docs_comments SET assigned_to = NULL WHERE assigned_to = $1",
        "UPDATE docs_updates SET user_id = '' WHERE user_id = $1",
        "UPDATE docs_versions SET created_by = '' WHERE created_by = $1",
        "UPDATE docs_reviews SET created_by = '' WHERE created_by = $1",
    ] {
        tx.execute(sql, &[user.into()]).await?;
    }
    for review in turns {
        advance(tx, &review).await?;
    }
    Ok(())
}

/// Documents and their owners, for the sweep.
/// Every document: its id, who made it, and the owner of its drive item
/// when known.
pub async fn documents(
    store: &dyn AsyncStore,
) -> StoreResult<Vec<(String, String, Option<String>)>> {
    Ok(store
        .query(
            "SELECT doc_id, owner_id, drive_owner FROM docs_documents ORDER BY doc_id",
            &[],
        )
        .await?
        .iter()
        .map(|r| {
            (
                r.text(0).unwrap_or_default().to_string(),
                r.text(1).unwrap_or_default().to_string(),
                r.text(2).map(str::to_string),
            )
        })
        .collect())
}

pub async fn set_drive_owner(store: &dyn AsyncStore, doc: &str, owner: &str) -> StoreResult<()> {
    store
        .execute(
            "UPDATE docs_documents SET drive_owner = $1 WHERE doc_id = $2",
            &[owner.into(), doc.into()],
        )
        .await?;
    Ok(())
}
