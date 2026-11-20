//! What someone who may only comment may change: comments and suggestions,
//! nothing of the text itself. Their editor writes Yjs updates like an
//! editor's; the server applies one to a copy of the document and refuses
//! it when the accepted text (suggested insertions left out, suggested
//! deletions kept, other marks counted) changes, or when a new suggestion
//! names someone else as its author.

use std::collections::BTreeSet;

use yrs::types::text::YChange;
use yrs::types::Attrs;
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, Out, ReadTxn, Text, Transact, Update, XmlFragment, XmlOut};

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

/// The accepted text, and every suggestion's (id, author).
#[derive(Default, PartialEq, Eq, Debug)]
struct View {
    text: String,
    suggestions: BTreeSet<(String, String)>,
}

fn attr_text(attrs: &Attrs, mark: &str, field: &str) -> Option<String> {
    match attrs.get(mark)? {
        Any::Map(m) => m.get(field).and_then(|v| match v {
            Any::String(s) => Some(s.to_string()),
            _ => None,
        }),
        _ => Some(String::new()),
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
                let attrs = d.attributes.as_deref().cloned().unwrap_or_default();
                for mark in ["insertion", "deletion"] {
                    if attrs.contains_key(mark) {
                        view.suggestions.insert((
                            attr_text(&attrs, mark, "id").unwrap_or_default(),
                            attr_text(&attrs, mark, "author").unwrap_or_default(),
                        ));
                    }
                }
                if attrs.contains_key("insertion") {
                    continue;
                }
                let mut marks: Vec<&str> = attrs
                    .keys()
                    .map(|k| k.as_ref())
                    // y-prosemirror keys marks that may overlap (comments)
                    // as "comment--<hash>".
                    .filter(|k| *k != "deletion" && *k != "comment" && !k.starts_with("comment--"))
                    .collect();
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
    out
}

/// Whether `update` from a commenter named `author` only comments and
/// suggests, given the document so far (`parts`: its checkpoint and the
/// updates after it).
pub fn commenter_may(parts: &[&[u8]], update: &[u8], author: &str) -> Result<(), String> {
    let before = view(&load(parts)?);
    let mut all = parts.to_vec();
    all.push(update);
    let after = view(&load(&all)?);
    if before.text != after.text {
        return Err("Someone who may comment suggests changes; the text itself stays.".into());
    }
    if after
        .suggestions
        .difference(&before.suggestions)
        .any(|(_, by)| by != author)
    {
        return Err("A suggestion is made in one's own name.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use yrs::types::xml::XmlElementPrelim;
    use yrs::{XmlElementRef, XmlTextPrelim, XmlTextRef};

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

    fn mark(name: &str, author: &str) -> Attrs {
        let mut inner = HashMap::new();
        inner.insert("id".to_string(), Any::from("s1"));
        inner.insert("author".to_string(), Any::from(author));
        let mut attrs = Attrs::new();
        attrs.insert(Arc::from(name), Any::Map(Arc::new(inner)));
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
    }
}
