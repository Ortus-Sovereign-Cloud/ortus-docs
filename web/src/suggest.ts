// Suggesting: while it is on, what someone types is marked as their
// insertion and what they remove is marked as their deletion instead of
// going. Each suggestion is accepted (it becomes the text) or rejected
// (it goes) by someone who may edit.
import type { Node as PMNode, Mark } from "prosemirror-model";
import { TextSelection, type EditorState, type Transaction } from "prosemirror-state";
import { ReplaceStep, type Step } from "prosemirror-transform";
import { schema } from "./schema.js";
import { ySyncPluginKey, yUndoPluginKey } from "y-prosemirror";

/** A change that came from someone else (or from undo): never rewritten. */
export function remote(tr: Transaction): boolean {
  return !!tr.getMeta(ySyncPluginKey) || !!tr.getMeta(yUndoPluginKey);
}

export interface Suggestion {
  id: string;
  author: string;
  at: number;
  inserted: string;
  deleted: string;
  from: number;
}

function newId(): string {
  return Math.random().toString(36).slice(2, 12);
}

/**
 * Turns an edit made while suggesting into marks: removed text stays,
 * marked as deleted (unless it was one's own suggestion, which simply
 * goes), and new text is marked as inserted.
 */
export function asSuggestion(state: EditorState, tr: Transaction, author: string): Transaction | null {
  if (!tr.docChanged || tr.getMeta("suggest") || remote(tr)) return null;
  if (!tr.steps.every((s: Step) => s instanceof ReplaceStep)) return null;
  const out = state.tr;
  // Typing on from one's own suggestion continues it (one suggestion, not
  // one per key).
  const first = (tr.steps[0] as unknown as { from: number }).from;
  const near = [state.doc.resolve(Math.min(first, state.doc.content.size)).nodeBefore, state.doc.resolve(Math.min(first, state.doc.content.size)).nodeAfter];
  const own = near.flatMap((n) => n?.marks ?? []).find((m) => (m.type === schema.marks.insertion || m.type === schema.marks.deletion) && m.attrs.author === author);
  const id = own ? (own.attrs.id as string) : newId();
  const at = own ? (own.attrs.at as number) : Date.now();
  const ins = schema.marks.insertion!.create({ id, author, at });
  const del = schema.marks.deletion!.create({ id, author, at });
  let cursor: number | null = null;
  for (const step of tr.steps as ReplaceStep[]) {
    const json = step.toJSON() as { from: number; to: number };
    const from = out.mapping.map(json.from);
    const to = out.mapping.map(json.to, -1);
    const slice = (step as unknown as { slice: { content: PMNode["content"]; size: number } }).slice;
    // What is removed: one's own fresh insertions go; the rest is marked.
    if (to > from) {
      const own: [number, number][] = [];
      out.doc.nodesBetween(from, to, (node, pos) => {
        if (node.isText && node.marks.some((m: Mark) => m.type === schema.marks.insertion && m.attrs.author === author)) {
          own.push([Math.max(pos, from), Math.min(pos + node.nodeSize, to)]);
        }
      });
      out.addMark(from, to, del);
      for (const [a, b] of own.reverse()) out.delete(a, b);
    }
    const insertAt = out.mapping.map(json.to);
    if (slice.size > 0) {
      out.insert(insertAt, slice.content);
      const end = insertAt + slice.size;
      out.removeMark(insertAt, end, schema.marks.deletion);
      out.addMark(insertAt, end, ins);
      cursor = end;
    } else {
      cursor = out.mapping.map(json.from);
    }
  }
  out.setMeta("suggest", true);
  if (cursor !== null) out.setSelection(TextSelection.near(out.doc.resolve(Math.min(cursor, out.doc.content.size))));
  return out;
}

/** Every suggestion in the document, in order. */
export function suggestions(doc: PMNode): Suggestion[] {
  const found = new Map<string, Suggestion>();
  doc.descendants((node, pos) => {
    if (!node.isText) return;
    for (const m of node.marks) {
      if (m.type !== schema.marks.insertion && m.type !== schema.marks.deletion) continue;
      const s = found.get(m.attrs.id) ?? { id: m.attrs.id, author: m.attrs.author, at: m.attrs.at, inserted: "", deleted: "", from: pos };
      if (m.type === schema.marks.insertion) s.inserted += node.text ?? "";
      else s.deleted += node.text ?? "";
      found.set(m.attrs.id, s);
    }
  });
  return [...found.values()].sort((a, b) => a.from - b.from);
}

/** Accepts (the insertion stays, the deletion goes) or rejects (the reverse). */
export function settle(state: EditorState, id: string, accept: boolean): Transaction {
  const tr = state.tr;
  const remove: [number, number][] = [];
  const keep: [number, number, Mark][] = [];
  state.doc.descendants((node, pos) => {
    if (!node.isText) return;
    for (const m of node.marks) {
      if (m.attrs.id !== id || (m.type !== schema.marks.insertion && m.type !== schema.marks.deletion)) continue;
      const range: [number, number] = [pos, pos + node.nodeSize];
      const goes = (m.type === schema.marks.deletion) === accept;
      if (goes) remove.push(range);
      else keep.push([...range, m]);
    }
  });
  // Kept text is written again without its mark: other editors render a
  // change of text reliably, a change of marks alone not always.
  for (const [a, b, m] of keep) {
    const node = state.doc.nodeAt(a);
    if (!node?.isText) continue;
    tr.replaceWith(tr.mapping.map(a), tr.mapping.map(b), schema.text(node.text ?? "", m.removeFromSet(node.marks)));
  }
  for (const [a, b] of remove.reverse()) tr.delete(tr.mapping.map(a), tr.mapping.map(b));
  tr.setMeta("suggest", true);
  return tr;
}
