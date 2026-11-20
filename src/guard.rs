//! What someone who may only comment may change: comments and suggestions,
//! nothing of the text itself. Their editor writes Yjs updates like an
//! editor's; the server applies one to a copy of the document and refuses
//! it when the accepted text (suggested insertions left out, suggested
//! deletions kept, other marks counted) changes, or when a new suggestion
//! names someone else as its author.

use std::collections::BTreeSet;

use yrs::types::text::YChange;
use yrs::types::Attrs;
use yrs::types::ToJson;
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, Out, ReadTxn, Text, Transact, Update, Xml, XmlFragment, XmlOut};

const FRAGMENT: &str = "prosemirror";

fn load(parts: &[&[u8]]) -> Result<Doc, String> {
    let doc = Doc::new();
    doc.get_or_insert_xml_fragment(FRAGMENT);
    {
        let mut txn = doc.transact_mut();
        for part in parts {
            let update = Update::decode_v1(part).map_err(|e| e.to_string())?;
            txn.apply_update(update).map_err(|e| e.to_string())?;
        }
    }
    Ok(doc)
}

/// The accepted text, every suggestion (id → author), every comment id,
/// and the document's other top-level types.
#[derive(Default, PartialEq, Eq, Debug)]
struct View {
    text: String,
    suggestions: BTreeSet<(String, String)>,
    comments: BTreeSet<String>,
    others: String,
}

/// The mark a formatting key stands for: y-prosemirror keys marks that may
/// overlap as "<name>--<hash>".
fn mark_name(key: &str) -> &str {
    key.split_once("--").map_or(key, |(name, _)| name)
}

fn attr_text(value: &Any, field: &str) -> String {
    match value {
        Any::Map(m) => m.get(field).map_or(String::new(), |v| match v {
            Any::String(s) => s.to_string(),
            other => other.to_string(),
        }),
        Any::String(s) => s.to_string(),
        _ => String::new(),
    }
}

fn walk<T: ReadTxn>(txn: &T, node: XmlOut, view: &mut View, depth: usize) {
    if depth > 200 {
        return;
    }
    match node {
        XmlOut::Element(el) => {
            view.text.push('<');
            view.text.push_str(el.tag());
            // Attributes count too (a heading's level, a list's start).
            let mut attrs: Vec<(String, String)> = el
                .attributes(txn)
                .map(|(k, v)| (k.to_string(), v.to_string(txn)))
                .collect();
            attrs.sort();
            for (k, v) in attrs {
                view.text.push_str(&format!(" {k}={v:?}"));
            }
            view.text.push('>');
            for child in el.children(txn) {
                walk(txn, child, view, depth + 1);
            }
            view.text.push_str("</>");
        }
        XmlOut::Fragment(f) => {
            for child in f.children(txn) {
                walk(txn, child, view, depth + 1);
            }
        }
        XmlOut::Text(t) => {
            for d in t.diff(txn, YChange::identity) {
                let attrs: Attrs = d.attributes.as_deref().cloned().unwrap_or_default();
                let mut inserted = false;
                let mut marks: Vec<String> = Vec::new();
                for (key, value) in attrs.iter() {
                    match mark_name(key) {
                        "insertion" | "deletion" => {
                            inserted |= mark_name(key) == "insertion";
                            view.suggestions
                                .insert((attr_text(value, "id"), attr_text(value, "author")));
                        }
                        "comment" => {
                            view.comments.insert(attr_text(value, "id"));
                        }
                        other => marks.push(format!("{other}={value}")),
                    }
                }
                if inserted {
                    continue;
                }
                marks.sort_unstable();
                if !marks.is_empty() {
                    view.text.push('[');
                    view.text.push_str(&marks.join(","));
                    view.text.push(']');
                }
                match d.insert {
                    Out::Any(Any::String(s)) => view.text.push_str(&s),
                    other => view.text.push_str(&other.to_string(txn)),
                }
            }
        }
    }
}

fn view(doc: &Doc) -> View {
    let fragment = doc.get_or_insert_xml_fragment(FRAGMENT);
    let txn = doc.transact();
    let mut out = View::default();
    for child in fragment.children(&txn) {
        walk(&txn, child, &mut out, 0);
    }
    // Anything kept beside the document's body: a commenter adds nothing.
    let mut others: Vec<String> = txn
        .root_refs()
        .filter(|(name, _)| *name != FRAGMENT)
        .map(|(name, value)| format!("{name}={}", value.to_json(&txn)))
        .collect();
    others.sort();
    out.others = others.join(";");
    out
}

/// Whether `update` from a commenter named `author` only comments and
/// suggests, given the document so far (`parts`: its checkpoint and the
/// updates after it): the accepted text stays, new suggestions are theirs
/// under new ids, and nobody else's suggestions or comments go.
pub fn commenter_may(parts: &[&[u8]], update: &[u8], author: &str) -> Result<(), String> {
    let before = view(&load(parts)?);
    let mut all = parts.to_vec();
    all.push(update);
    let after = view(&load(&all)?);
    if before.text != after.text || before.others != after.others {
        return Err("Someone who may comment suggests changes; the text itself stays.".into());
    }
    let known: std::collections::BTreeMap<&str, &str> = before
        .suggestions
        .iter()
        .map(|(id, by)| (id.as_str(), by.as_str()))
        .collect();
    for (id, by) in after.suggestions.difference(&before.suggestions) {
        // A new suggestion: theirs, and not under someone else's id.
        if by != author || known.get(id.as_str()).is_some_and(|other| *other != author) {
            return Err("A suggestion is made in one's own name.".into());
        }
    }
    let kept: BTreeSet<&str> = after
        .suggestions
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();
    if before
        .suggestions
        .iter()
        .any(|(id, by)| by != author && !kept.contains(id.as_str()))
    {
        return Err("Someone else's suggestion stays until an editor decides.".into());
    }
    if !before.comments.is_subset(&after.comments) {
        return Err("Comments stay until an editor resolves them.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use yrs::types::xml::XmlElementPrelim;
    use yrs::{Map, XmlElementRef, XmlTextPrelim, XmlTextRef};

    /// A document with one paragraph, as an editor wrote it.
    fn base() -> (Doc, Vec<u8>) {
        let doc = Doc::with_client_id(1);
        let f = doc.get_or_insert_xml_fragment(FRAGMENT);
        {
            let mut txn = doc.transact_mut();
            let p: XmlElementRef = f.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            p.push_back(&mut txn, XmlTextPrelim::new("Shelters: school, gym"));
        }
        let update = doc
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default());
        (doc, update)
    }

    fn text(doc: &Doc) -> XmlTextRef {
        let f = doc.get_or_insert_xml_fragment(FRAGMENT);
        let txn = doc.transact();
        let XmlOut::Element(p) = f.get(&txn, 0).unwrap() else {
            panic!()
        };
        let XmlOut::Text(t) = p.get(&txn, 0).unwrap() else {
            panic!()
        };
        t
    }

    /// A change by a second person (client 2), as an update.
    fn change(base: &[u8], edit: impl FnOnce(&Doc, &XmlTextRef)) -> Vec<u8> {
        let doc = Doc::with_client_id(2);
        doc.transact_mut()
            .apply_update(Update::decode_v1(base).unwrap())
            .unwrap();
        let before = doc.transact().state_vector();
        let t = text(&doc);
        edit(&doc, &t);
        let diff = doc.transact().encode_diff_v1(&before);
        diff
    }

    /// A suggestion mark as y-prosemirror writes it ("insertion--<hash>"
    /// for marks that may overlap).
    fn mark(name: &str, author: &str) -> Attrs {
        mark_with(name, "s1", author)
    }

    fn mark_with(name: &str, id: &str, author: &str) -> Attrs {
        let mut inner = HashMap::new();
        inner.insert("id".to_string(), Any::from(id));
        inner.insert("author".to_string(), Any::from(author));
        let mut attrs = Attrs::new();
        attrs.insert(
            Arc::from(format!("{name}--k{id}").as_str()),
            Any::Map(Arc::new(inner)),
        );
        attrs
    }

    #[test]
    fn commenters_comment_and_suggest_but_do_not_write() {
        let (_, b) = base();
        // A comment on words: allowed.
        let comment = change(&b, |doc, t| {
            let mut attrs = Attrs::new();
            attrs.insert(Arc::from("comment"), Any::from("c1"));
            t.format(&mut doc.transact_mut(), 0, 9, attrs);
        });
        assert!(commenter_may(&[&b], &comment, "Marek Nowak").is_ok());
        // A suggested insertion and deletion in one's own name: allowed.
        let suggest = change(&b, |doc, t| {
            let mut txn = doc.transact_mut();
            t.insert_with_attributes(&mut txn, 21, ", church", mark("insertion", "Marek Nowak"));
            t.format(&mut txn, 18, 3, mark("deletion", "Marek Nowak"));
        });
        assert!(commenter_may(&[&b], &suggest, "Marek Nowak").is_ok());
        // Writing straight into the text: refused.
        let write = change(&b, |doc, t| t.insert(&mut doc.transact_mut(), 0, "No "));
        assert!(commenter_may(&[&b], &write, "Marek Nowak").is_err());
        let remove = change(&b, |doc, t| t.remove_range(&mut doc.transact_mut(), 0, 9));
        assert!(commenter_may(&[&b], &remove, "Marek Nowak").is_err());
        // Bold is a change of the text too.
        let bold = change(&b, |doc, t| {
            let mut attrs = Attrs::new();
            attrs.insert(Arc::from("strong"), Any::Bool(true));
            t.format(&mut doc.transact_mut(), 0, 9, attrs);
        });
        assert!(commenter_may(&[&b], &bold, "Marek Nowak").is_err());
        // A suggestion in someone else's name: refused.
        let forged = change(&b, |doc, t| {
            t.insert_with_attributes(
                &mut doc.transact_mut(),
                21,
                ", church",
                mark("insertion", "Olena Hrytsenko"),
            );
        });
        assert!(commenter_may(&[&b], &forged, "Marek Nowak").is_err());
        // Olena's suggestion is in the document; Marek may not add text
        // under her suggestion's id, nor take her suggestion away.
        let hers = change(&b, |doc, t| {
            t.insert_with_attributes(
                &mut doc.transact_mut(),
                21,
                ", church",
                mark_with("insertion", "o1", "Olena Hrytsenko"),
            );
        });
        let under_hers = {
            let doc = Doc::with_client_id(3);
            for u in [&b, &hers] {
                doc.transact_mut()
                    .apply_update(Update::decode_v1(u).unwrap())
                    .unwrap();
            }
            let before = doc.transact().state_vector();
            let t = text(&doc);
            t.insert_with_attributes(
                &mut doc.transact_mut(),
                0,
                "No ",
                mark_with("insertion", "o1", "Marek Nowak"),
            );
            let diff = doc.transact().encode_diff_v1(&before);
            diff
        };
        assert!(commenter_may(&[&b, &hers], &under_hers, "Marek Nowak").is_err());
        let removed = {
            let doc = Doc::with_client_id(4);
            for u in [&b, &hers] {
                doc.transact_mut()
                    .apply_update(Update::decode_v1(u).unwrap())
                    .unwrap();
            }
            let before = doc.transact().state_vector();
            let t = text(&doc);
            t.remove_range(&mut doc.transact_mut(), 21, 8);
            let diff = doc.transact().encode_diff_v1(&before);
            diff
        };
        assert!(commenter_may(&[&b, &hers], &removed, "Marek Nowak").is_err());
        // Nothing beside the body.
        let extra = change(&b, |doc, _| {
            let m = doc.get_or_insert_map("notes");
            m.insert(&mut doc.transact_mut(), "x", "y");
        });
        assert!(commenter_may(&[&b], &extra, "Marek Nowak").is_err());
    }
}
