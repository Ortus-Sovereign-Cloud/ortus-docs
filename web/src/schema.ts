// The document's structure (ProseMirror): headings, paragraphs, lists,
// tables, bold and italic; and three marks of collaboration: an
// insertion and a deletion someone suggested, and a comment's place.
import { Schema, type MarkSpec } from "prosemirror-model";
import { schema as basic } from "prosemirror-schema-basic";
import { addListNodes } from "prosemirror-schema-list";
import { tableNodes } from "prosemirror-tables";

const insertion: MarkSpec = {
  attrs: { id: { default: "" }, author: { default: "" }, at: { default: 0 } },
  inclusive: true,
  excludes: "deletion",
  parseDOM: [{ tag: "ins", getAttrs: (d) => ({ id: (d as HTMLElement).dataset.id ?? "", author: (d as HTMLElement).dataset.author ?? "" }) }],
  toDOM: (m) => ["ins", { class: "suggest-in", "data-id": m.attrs.id, "data-author": m.attrs.author }, 0],
};

const deletion: MarkSpec = {
  attrs: { id: { default: "" }, author: { default: "" }, at: { default: 0 } },
  inclusive: false,
  excludes: "insertion",
  parseDOM: [{ tag: "del", getAttrs: (d) => ({ id: (d as HTMLElement).dataset.id ?? "", author: (d as HTMLElement).dataset.author ?? "" }) }],
  toDOM: (m) => ["del", { class: "suggest-out", "data-id": m.attrs.id, "data-author": m.attrs.author }, 0],
};

const comment: MarkSpec = {
  attrs: { id: { default: "" } },
  inclusive: false,
  excludes: "",
  parseDOM: [{ tag: "mark.comment", getAttrs: (d) => ({ id: (d as HTMLElement).dataset.id ?? "" }) }],
  toDOM: (m) => ["mark", { class: "comment", "data-id": m.attrs.id }, 0],
};

const nodes = addListNodes(basic.spec.nodes, "paragraph block*", "block").append(tableNodes({ tableGroup: "block", cellContent: "block+", cellAttributes: {} }));

export const schema = new Schema({
  nodes,
  marks: basic.spec.marks.remove("link").remove("code").addToEnd("insertion", insertion).addToEnd("deletion", deletion).addToEnd("comment", comment),
});
