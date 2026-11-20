// Word and PDF: a document out as a .docx (the text as it stands, pending
// suggestions not applied) or printed to PDF by the browser, and a .docx
// in as a new document. The Word libraries load only when used.
import { call } from "ortus-kit";
import { DOMParser as PMParser, type Node as PMNode } from "prosemirror-model";
import { prosemirrorToYDoc } from "y-prosemirror";
import * as Y from "yjs";
import { schema } from "./schema.js";

interface Json {
  type: string;
  text?: string;
  attrs?: { level?: number };
  marks?: { type: string }[];
  content?: Json[];
}

const RTL = /[֐-ࣿ]/;

/** Pending insertions are left out; pending deletions are still text. */
function runs(nodes: Json[] = []): Json[] {
  return nodes.filter((n) => n.type === "text" && !n.marks?.some((m) => m.type === "insertion"));
}

function plain(node: Json): string {
  return node.type === "text" ? (runs([node])[0]?.text ?? "") : (node.content ?? []).map(plain).join("");
}

function fileName(name: string, ext: string): string {
  return `${name.replace(/[\\/:*?"<>|]+/g, " ").trim() || "document"}.${ext}`;
}

function save(blob: Blob, name: string): void {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
  window.setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
}

/** The document as a Word file. */
export async function exportDocx(name: string, doc: Json): Promise<Blob> {
  const d = await import("docx");
  const text = (nodes?: Json[]) =>
    runs(nodes).map((n) => new d.TextRun({ text: n.text ?? "", bold: !!n.marks?.some((m) => m.type === "strong"), italics: !!n.marks?.some((m) => m.type === "em") }));
  const levels = [d.HeadingLevel.HEADING_1, d.HeadingLevel.HEADING_2, d.HeadingLevel.HEADING_3];
  const blocks = (nodes: Json[] = [], list?: { ordered: boolean; level: number }): (InstanceType<typeof d.Paragraph> | InstanceType<typeof d.Table>)[] =>
    nodes.flatMap((n) => {
      const bidirectional = RTL.test(plain(n));
      switch (n.type) {
        case "heading":
          return [new d.Paragraph({ heading: levels[(n.attrs?.level ?? 1) - 1] ?? d.HeadingLevel.HEADING_3, bidirectional, children: text(n.content) })];
        case "paragraph":
          return [
            new d.Paragraph({
              bidirectional,
              children: text(n.content),
              ...(list ? (list.ordered ? { numbering: { reference: "ordered", level: list.level } } : { bullet: { level: list.level } }) : {}),
            }),
          ];
        case "bullet_list":
        case "ordered_list":
          return (n.content ?? []).flatMap((li) => blocks(li.content, { ordered: n.type === "ordered_list", level: list ? Math.min(list.level + 1, 8) : 0 }));
        case "table":
          return [
            new d.Table({
              width: { size: 100, type: d.WidthType.PERCENTAGE },
              rows: (n.content ?? []).map(
                (row) =>
                  new d.TableRow({
                    children: (row.content ?? []).map((cell) => new d.TableCell({ children: blocks(cell.content).filter((b) => b instanceof d.Paragraph) as InstanceType<typeof d.Paragraph>[] })),
                  }),
              ),
            }),
          ];
        default:
          return [];
      }
    });
  const file = new d.Document({
    title: name,
    numbering: { config: [{ reference: "ordered", levels: Array.from({ length: 9 }, (_, level) => ({ level, format: d.LevelFormat.DECIMAL, text: `%${level + 1}.`, alignment: d.AlignmentType.START })) }] },
    sections: [{ children: blocks(doc.content) }],
  });
  return d.Packer.toBlob(file);
}

export async function downloadDocx(name: string, doc: Json): Promise<void> {
  save(await exportDocx(name, doc), fileName(name, "docx"));
}

/**
 * The document alone in a new window, printed: the browser's "Save as
 * PDF" makes the file (its fonts cover every language the suite has).
 */
export function printPdf(name: string, page: HTMLElement): Window | null {
  const w = window.open("", "_blank");
  if (!w) return null;
  const d = w.document;
  d.title = name;
  d.documentElement.lang = document.documentElement.lang;
  d.documentElement.dir = document.documentElement.dir;
  const sheet = new (w as unknown as typeof globalThis).CSSStyleSheet();
  sheet.replaceSync(`
    body { margin: 0; font-family: "Source Serif 4", Georgia, serif; font-size: 12pt; line-height: 1.5; color: #000; background: #fff; }
    main { max-inline-size: 170mm; margin-inline: auto; padding-block: 12mm; }
    h1 { font-size: 22pt; } h2 { font-size: 15pt; }
    table { inline-size: 100%; border-collapse: collapse; font-family: system-ui, sans-serif; font-size: 10.5pt; }
    td, th { border: 1px solid #999; padding-block: 4pt; padding-inline: 6pt; text-align: start; vertical-align: top; }
    td p, th p { margin: 0; }
    ins { display: none; } del { text-decoration: none; }
    mark { background: none; }
    .ProseMirror-yjs-cursor, .ProseMirror-widget, .ProseMirror-separator, .ProseMirror-trailingBreak, .column-resize-handle { display: none; }
    @page { margin: 18mm; }
  `);
  d.adoptedStyleSheets = [sheet];
  const main = d.createElement("main");
  const root = page.classList.contains("ProseMirror") ? page : page.querySelector(".ProseMirror");
  const copy = root?.cloneNode(true) as HTMLElement | undefined;
  if (copy) {
    copy.removeAttribute("contenteditable");
    main.append(...Array.from(copy.childNodes).map((n) => d.importNode(n, true)));
  }
  d.body.append(main);
  w.setTimeout(() => w.print(), 50);
  return w;
}

/** A Word file read into the document structure. */
export async function importDocx(file: File): Promise<PMNode> {
  const mammoth = await import("mammoth");
  const { value } = await mammoth.convertToHtml({ arrayBuffer: await file.arrayBuffer() });
  // Parsed in an inert document: nothing in it runs or loads.
  const inert = new window.DOMParser().parseFromString(`<body>${value}</body>`, "text/html");
  for (const img of Array.from(inert.querySelectorAll("img"))) img.remove();
  return PMParser.fromSchema(schema).parse(inert.body);
}

/** A new document from a Word file; its address. */
export async function newFromDocx(file: File): Promise<string> {
  const doc = await importDocx(file);
  const made = await call<{ id: string; path: string }>("POST", "/docs/documents", { name: file.name.replace(/\.docx$/i, "") });
  const ydoc = prosemirrorToYDoc(doc, "prosemirror");
  const r = await fetch(`/api/v1/docs/documents/${encodeURIComponent(made.id)}/updates?client=import`, {
    method: "POST",
    headers: { "X-Requested-With": "ortus", "Content-Type": "application/octet-stream" },
    body: new Blob([Y.encodeStateAsUpdate(ydoc) as BlobPart]),
  });
  if (!r.ok) throw new Error(await r.text());
  const { seq } = (await r.json()) as { seq: number };
  let s = "";
  const bytes = Y.encodeStateAsUpdate(ydoc);
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  await call("POST", `/docs/documents/${encodeURIComponent(made.id)}/checkpoint`, { seq, state: btoa(s), doc: doc.toJSON() });
  return made.path;
}
