// The editor (board WebDocEditor): the page shared live with everyone in
// it (their cursors, their changes), editing or suggesting, comments on
// a passage, the suggestions to accept or reject, sending for approval.
import { ApiError, LitElement, LocalizeController, call, controls, css, customElement, html, nothing, query, state } from "ortus-kit";
import { baseKeymap, setBlockType, toggleMark } from "prosemirror-commands";
import { keymap } from "prosemirror-keymap";
import { EditorState, TextSelection, type Transaction } from "prosemirror-state";
import { addRowAfter, columnResizing, goToNextCell, tableEditing } from "prosemirror-tables";
import { EditorView } from "prosemirror-view";
import { redo, undo, yCursorPlugin, ySyncPlugin, yUndoPlugin } from "y-prosemirror";
import { t } from "./i18n.js";
import type { DocsKey } from "./locales/en.js";
import { schema } from "./schema.js";
import { asSuggestion, remote, settle, suggestions, type Suggestion } from "./suggest.js";
import { Sync, type Here, type Opened } from "./sync.js";
import { downloadDocx, printPdf } from "./word.js";

interface Comment {
  id: string;
  thread: string;
  author: string;
  body: string;
  quote: string;
  assignedTo: string | null;
  resolved: boolean;
  at: number;
}

type Mode = "editing" | "suggesting" | "viewing";

// y-prosemirror asks the editor's root for a range; the editor lives in a
// shadow root, which has none of its own: the document's serves.
const shadowProto = ShadowRoot.prototype as unknown as { createRange?: () => Range };
if (!shadowProto.createRange) shadowProto.createRange = () => document.createRange();

const TONES = ["#a5b4ff", "#f9b77a", "#6ee0d0", "#d4f27a", "#ffa39a", "#eceef3"];

function tone(text: string): string {
  let h = 0;
  for (const c of text) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return TONES[h % TONES.length]!;
}

function initials(name: string): string {
  const w = name.split(/[\s@.]+/).filter(Boolean);
  return ((w[0]?.[0] ?? "?") + (w.length > 1 ? (w[1]?.[0] ?? "") : "")).toUpperCase();
}

function ago(ms: number, lang: string): string {
  const minutes = Math.round((Date.now() - ms) / 60_000);
  return new Intl.RelativeTimeFormat(lang, { numeric: "auto" }).format(-Math.max(0, minutes), "minute");
}

@customElement("ortus-docs-editor")
export class OrtusDocsEditor extends LitElement {
  static override styles = [
    controls,
    css`
      :host {
        display: grid;
        grid-template-rows: auto auto 1fr;
        min-block-size: 100vh;
        color: var(--ortus-text);
      }
      header {
        display: flex;
        align-items: center;
        gap: 12px;
        padding-block: 12px;
        padding-inline: 24px;
        border-block-end: 1px solid var(--ortus-line);
      }
      .back {
        inline-size: 36px;
        block-size: 36px;
        border: 0;
        background: none;
        color: var(--ortus-text);
        font-size: 18px;
        cursor: pointer;
      }
      .title {
        display: grid;
        min-inline-size: 0;
      }
      .title input {
        border: 0;
        background: transparent;
        color: var(--ortus-text);
        font: inherit;
        font-size: 16px;
        font-weight: 600;
        padding: 0;
        inline-size: 320px;
      }
      .title small {
        color: var(--ortus-text-3);
        font-size: 13px;
      }
      .chip {
        padding-block: 3px;
        padding-inline: 8px;
        border-radius: 6px;
        font-size: 12px;
        font-weight: 600;
        background: var(--ortus-surface-2);
        color: var(--ortus-text-2);
      }
      .chip.label {
        font-family: var(--ortus-mono, monospace);
        background: color-mix(in srgb, var(--ortus-orange) 18%, transparent);
        color: var(--ortus-orange);
      }
      .spacer {
        flex: 1;
      }
      .people {
        display: flex;
      }
      .people span {
        display: grid;
        place-items: center;
        inline-size: 34px;
        block-size: 34px;
        border-radius: 10px;
        margin-inline-start: -6px;
        border: 2px solid var(--ortus-bg);
        color: #0a0b10;
        font-size: 12px;
        font-weight: 700;
      }
      select.mode {
        block-size: 40px;
        padding-inline: 12px;
        border-radius: 12px;
        border: 1px solid var(--ortus-line-strong);
        background: var(--ortus-surface);
        color: var(--ortus-text);
        font: inherit;
        font-weight: 600;
      }
      .tools {
        display: flex;
        gap: 6px;
        align-items: center;
        padding-block: 10px;
        padding-inline: 24px;
        border-block-end: 1px solid var(--ortus-line);
      }
      .tools button,
      .tools select {
        block-size: 36px;
        padding-inline: 12px;
        border: 0;
        border-radius: 10px;
        background: transparent;
        color: var(--ortus-text);
        font: inherit;
        font-weight: 600;
        cursor: pointer;
      }
      .tools select {
        background: var(--ortus-surface);
      }
      .tools button[aria-pressed="true"] {
        background: var(--ortus-hover);
      }
      .body {
        display: grid;
        grid-template-columns: minmax(0, 1fr) 320px;
        gap: 32px;
        padding-block: 32px;
        padding-inline: 112px 24px;
        align-items: start;
      }
      .page {
        min-block-size: 70vh;
        padding-block: 56px;
        padding-inline: 64px;
        border-radius: 6px;
        background: #fafaf8;
        color: #1b1d24;
        font-family: "Source Serif 4", Georgia, serif;
        font-size: 17px;
        line-height: 1.6;
      }
      .ProseMirror {
        outline: none;
        white-space: pre-wrap;
        word-wrap: break-word;
        position: relative;
      }
      .ProseMirror h1 {
        font-size: 30px;
        margin-block: 0 24px;
      }
      .ProseMirror h2 {
        font-size: 20px;
        margin-block: 24px 12px;
      }
      .ProseMirror p {
        margin-block: 0 14px;
      }
      .ProseMirror table {
        border-collapse: collapse;
        inline-size: 100%;
        font-family: var(--ortus-font);
        font-size: 15px;
        margin-block: 12px;
      }
      .ProseMirror td,
      .ProseMirror th {
        border: 1px solid #c9ccd4;
        padding-block: 10px;
        padding-inline: 14px;
        vertical-align: top;
        text-align: start;
      }
      .ProseMirror th {
        background: #eef0f4;
      }
      .ProseMirror td p,
      .ProseMirror th p {
        margin: 0;
      }
      ins.suggest-in {
        color: #1d7a55;
        text-decoration: underline;
      }
      del.suggest-out {
        color: #c0392b;
      }
      mark.comment {
        background: #fde9a6;
        color: inherit;
      }
      .ProseMirror-yjs-cursor {
        position: relative;
        border-inline-start: 2px solid;
        margin-inline: -1px;
      }
      .ProseMirror-yjs-selection {
        background: rgb(165 180 255 / 0.35);
      }
      .ProseMirror-yjs-cursor > div {
        position: absolute;
        inset-block-end: 100%;
        inset-inline-start: -1px;
        padding-block: 0;
        padding-inline: 6px;
        border-radius: 4px;
        font-family: var(--ortus-font);
        font-size: 12px;
        font-weight: 600;
        color: #fff;
        white-space: nowrap;
      }
      aside {
        display: grid;
        gap: 16px;
      }
      .card {
        display: grid;
        gap: 10px;
        padding: 20px;
        border-radius: 18px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
        font-size: 14px;
      }
      .card p {
        margin: 0;
        line-height: 1.5;
      }
      .card small {
        color: var(--ortus-text-3);
      }
      .card .who {
        font-size: 13px;
        color: var(--ortus-text-3);
      }
      .card .who b {
        color: var(--ortus-text);
      }
      .row {
        display: flex;
        gap: 8px;
      }
      .row .button {
        flex: 1;
      }
      .assigned {
        color: var(--ortus-accent);
      }
      textarea {
        inline-size: 100%;
        box-sizing: border-box;
      }
      dialog {
        inline-size: min(720px, 94vw);
        padding: 0;
        border-radius: 22px;
        border: 1px solid var(--ortus-line-strong);
        background: var(--ortus-surface);
        color: var(--ortus-text);
      }
      dialog::backdrop {
        background: rgb(0 0 0 / 0.55);
      }
      dialog form {
        display: grid;
        gap: 14px;
        padding: 26px;
      }
      dialog h2 {
        margin: 0;
        font-size: 20px;
      }
      dialog label {
        display: grid;
        gap: 6px;
        font-size: 13px;
        color: var(--ortus-text-2);
      }
      .steps {
        display: grid;
        gap: 10px;
      }
      .step {
        display: grid;
        grid-template-columns: 1fr 1fr 150px 110px;
        gap: 8px;
      }
      textarea {
        font: inherit;
      }
      .error {
        margin: 0;
        color: var(--ortus-red);
        font-size: 14px;
      }
    `,
  ];

  private readonly localize = new LocalizeController(this);
  @state() private opened: Opened | null = null;
  @state() private mode: Mode = "editing";
  @state() private people: Here[] = [];
  @state() private saved = true;
  @state() private list: Suggestion[] = [];
  @state() private comments: Comment[] = [];
  @state() private error = "";
  @state() private draftComment: { quote: string; from: number; to: number } | null = null;
  @state() private steps: { email: string; title: string; due: string; kind: "approve" | "sign" }[] = [{ email: "", title: "", due: "", kind: "approve" }];
  @state() private caseRef = "";
  /** An administrator with the records app may publish a must-read. */
  @state() private canPublish = false;
  @state() private published = false;
  @query(".page") private page!: HTMLElement;
  private view: EditorView | null = null;
  private sync: Sync | null = null;

  private docId(): string {
    return new URLSearchParams(location.search).get("doc") ?? "";
  }

  override connectedCallback(): void {
    super.connectedCallback();
    // Moved within the page (the shell re-attaches app elements): the
    // editor carries on as it was.
    if (this.view) return;
    void this.load();
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    // Only a real departure ends the editor.
    window.setTimeout(() => {
      if (this.isConnected) return;
      this.sync?.close();
      this.view?.destroy();
      this.sync = null;
      this.view = null;
    }, 0);
  }

  private canEdit(): boolean {
    return this.opened?.role === "edit" || this.opened?.role === "owner";
  }

  private async load(): Promise<void> {
    try {
      const opened = await call<Opened>("GET", `/docs/documents/${encodeURIComponent(this.docId())}`);
      this.opened = opened;
      this.mode = this.canEdit() ? "editing" : opened.role === "comment" ? "suggesting" : "viewing";
      await this.updateComplete;
      this.mount(opened);
      await this.loadComments();
      const [me, apps] = await Promise.all([call<{ role: string }>("GET", "/me"), call<{ apps: { id: string }[] }>("GET", "/apps")]);
      this.canPublish = (me.role === "admin" || me.role === "owner") && apps.apps.some((a) => a.id === "records");
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
    }
  }

  private async loadComments(): Promise<void> {
    this.comments = (await call<{ comments: Comment[] }>("GET", `/docs/documents/${encodeURIComponent(this.docId())}/comments`)).comments;
  }

  private mount(opened: Opened): void {
    const writer = opened.role !== "view";
    const sync = new Sync(opened.id, writer, () => this.view?.state.doc.toJSON(), this.canEdit());
    this.sync = sync;
    sync.onPeople = (list) => (this.people = list);
    sync.onSaved = (saved) => (this.saved = saved);
    sync.onGone = () => location.reload();
    sync.start(opened);
    const fragment = sync.doc.getXmlFragment("prosemirror");
    sync.awareness.setLocalStateField("user", { name: opened.me.name, color: tone(opened.me.name) });
    const view = new EditorView(
      { mount: this.page },
      {
        state: EditorState.create({
          schema,
          plugins: [
            ySyncPlugin(fragment),
            // Cursors drawn with style properties (the CSP refuses style attributes).
            yCursorPlugin(sync.awareness, {
              cursorBuilder: (user: { name?: string; color?: string }) => {
                const caret = document.createElement("span");
                caret.className = "ProseMirror-yjs-cursor";
                caret.style.borderColor = user.color ?? "#a5b4ff";
                const label = document.createElement("div");
                label.style.backgroundColor = user.color ?? "#a5b4ff";
                label.textContent = user.name ?? "";
                caret.append(document.createTextNode("\u2060"), label, document.createTextNode("\u2060"));
                return caret;
              },
              selectionBuilder: () => ({ class: "ProseMirror-yjs-selection" }),
            }),
            yUndoPlugin(),
            keymap({ "Mod-z": undo, "Mod-y": redo, "Mod-Shift-z": redo, "Mod-b": toggleMark(schema.marks.strong!), "Mod-i": toggleMark(schema.marks.em!), Tab: (state, dispatch) => goToNextCell(1)(state, dispatch) || (addRowAfter(state, dispatch && ((tr) => dispatch(tr))) && this.view !== null && goToNextCell(1)(this.view.state, dispatch)), "Shift-Tab": goToNextCell(-1) }),
            keymap(baseKeymap),
            columnResizing(),
            tableEditing(),
          ],
        }),
        editable: () => this.mode !== "viewing",
        // The view the transaction is for (it may still be in construction).
        dispatchTransaction: ((host) =>
          function (this: EditorView, tr: Transaction) {
            host.dispatch(tr, this);
          })(this),
        attributes: { "aria-label": opened.name, role: "textbox", "aria-multiline": "true" },
      },
    );
    this.view = view;
    this.list = suggestions(view.state.doc);
  }

  private dispatch(tr: Transaction, on?: EditorView): void {
    const view = on ?? this.view;
    if (!view) return;
    let applied = tr;
    if (this.mode === "suggesting") {
      const rewritten = asSuggestion(view.state, tr, this.opened?.me.name ?? "");
      if (rewritten) applied = rewritten;
    } else if (this.opened?.role === "comment" && tr.docChanged && !remote(tr) && !tr.getMeta("suggest") && !tr.getMeta("comment")) {
      // Commenters change the text only by suggesting.
      return;
    }
    view.updateState(view.state.apply(applied));
    if (applied.docChanged) this.list = suggestions(view.state.doc);
  }

  private command(cmd: (state: EditorState, dispatch?: (tr: Transaction) => void) => boolean): void {
    if (!this.view) return;
    cmd(this.view.state, (tr) => this.dispatch(tr));
    this.view.focus();
  }

  private block(kind: string): void {
    if (kind === "p") this.command(setBlockType(schema.nodes.paragraph!));
    else this.command(setBlockType(schema.nodes.heading!, { level: Number(kind) }));
  }

  private insertTable(): void {
    const { table, table_row: row, table_cell: cell, table_header: head, paragraph } = schema.nodes;
    const cells = (type: typeof cell, n: number) => Array.from({ length: n }, () => type!.create(null, paragraph!.create()));
    const node = table!.create(null, [row!.create(null, cells(head, 3)), row!.create(null, cells(cell, 3)), row!.create(null, cells(cell, 3))]);
    this.command((state, dispatch) => {
      const from = state.selection.from;
      const tr = state.tr.replaceSelectionWith(node);
      // The cursor goes to the first cell.
      let at = -1;
      tr.doc.nodesBetween(Math.max(0, from - 2), tr.doc.content.size, (n, pos) => {
        if (at < 0 && n.type === table) at = pos;
        return at < 0;
      });
      if (at >= 0) tr.setSelection(TextSelection.near(tr.doc.resolve(at + 1)));
      dispatch?.(tr.scrollIntoView());
      return true;
    });
  }

  private startComment(): void {
    const view = this.view;
    if (!view) return;
    const { from, to } = view.state.selection;
    if (from === to) {
      this.error = t(this.localize, "selectFirst");
      return;
    }
    this.error = "";
    this.draftComment = { quote: view.state.doc.textBetween(from, to, " ").slice(0, 300), from, to };
  }

  private async saveComment(e: Event): Promise<void> {
    e.preventDefault();
    const draft = this.draftComment;
    const view = this.view;
    if (!draft || !view) return;
    const form = e.target as HTMLFormElement;
    const body = (form.querySelector("textarea") as HTMLTextAreaElement).value;
    const assign = (form.querySelector("input") as HTMLInputElement).value;
    try {
      const r = await call<{ thread: string; comments: Comment[] }>("POST", `/docs/documents/${encodeURIComponent(this.docId())}/comments`, { body, quote: draft.quote, assign: assign || undefined });
      const tr = view.state.tr.addMark(draft.from, draft.to, schema.marks.comment!.create({ id: r.thread })).setMeta("comment", true);
      this.dispatch(tr);
      this.comments = r.comments;
      this.draftComment = null;
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async reply(thread: string, form: HTMLFormElement): Promise<void> {
    const body = (form.querySelector("input") as HTMLInputElement).value;
    if (!body.trim()) return;
    const r = await call<{ comments: Comment[] }>("POST", `/docs/documents/${encodeURIComponent(this.docId())}/comments`, { thread, body });
    this.comments = r.comments;
    form.reset();
  }

  private async resolve(thread: string): Promise<void> {
    this.comments = (await call<{ comments: Comment[] }>("POST", `/docs/documents/${encodeURIComponent(this.docId())}/comments/${thread}/resolve`)).comments;
  }

  private decide(s: Suggestion, accept: boolean): void {
    if (!this.view) return;
    this.dispatch(settle(this.view.state, s.id, accept));
  }

  private async rename(name: string): Promise<void> {
    if (!name.trim() || name === this.opened?.name) return;
    try {
      await call("PATCH", `/docs/documents/${encodeURIComponent(this.docId())}`, { name });
      if (this.opened) this.opened = { ...this.opened, name };
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async sendForApproval(e: Event): Promise<void> {
    e.preventDefault();
    if (this.list.length) {
      this.error = t(this.localize, "settleFirst");
      return;
    }
    try {
      await this.sync?.settle();
      const steps = this.steps.filter((s) => s.email.trim()).map((s) => ({ email: s.email.trim(), title: s.title.trim(), dueAt: s.due ? Date.parse(s.due) : undefined, kind: s.kind }));
      await call("POST", `/docs/documents/${encodeURIComponent(this.docId())}/reviews`, { steps, case: this.caseRef.trim() || undefined });
      this.renderRoot.querySelector<HTMLDialogElement>("dialog.approval")?.close();
      const opened = await call<Opened>("GET", `/docs/documents/${encodeURIComponent(this.docId())}`);
      if (this.opened) this.opened = { ...this.opened, review: opened.review };
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async publish(e: Event): Promise<void> {
    e.preventDefault();
    const form = e.target as HTMLFormElement;
    const groups = (form.elements.namedItem("groups") as HTMLInputElement).value.split(",").map((g) => g.trim()).filter(Boolean);
    const note = (form.elements.namedItem("note") as HTMLInputElement).value;
    try {
      await call("POST", "/records/must-read", { title: this.opened?.name, note, body: this.view?.state.doc.toJSON(), source: `/docs?doc=${encodeURIComponent(this.docId())}`, groups });
      this.published = true;
      this.renderRoot.querySelector<HTMLDialogElement>("dialog.publish")?.close();
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private async withdraw(): Promise<void> {
    const review = this.opened?.review as { id: string } | null | undefined;
    if (!review) return;
    try {
      await call("POST", `/docs/reviews/${encodeURIComponent(review.id)}/cancel`);
      const opened = await call<Opened>("GET", `/docs/documents/${encodeURIComponent(this.docId())}`);
      if (this.opened) this.opened = { ...this.opened, review: opened.review };
    } catch (err) {
      this.error = err instanceof ApiError ? err.message : String(err);
    }
  }

  private isActive(mark: "strong" | "em"): boolean {
    const st = this.view?.state;
    if (!st) return false;
    const type = schema.marks[mark]!;
    const { from, to, empty, $from } = st.selection;
    return empty ? !!type.isInSet(st.storedMarks ?? $from.marks()) : st.doc.rangeHasMark(from, to, type);
  }

  override render() {
    const o = this.opened;
    if (!o) return html`${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}`;
    const lang = document.documentElement.lang || "en";
    const review = o.review as { state: string } | null | undefined;
    const stateKey: DocsKey = review?.state === "open" ? "inReview" : review?.state === "approved" ? "approved" : "draft";
    const modes: [Mode, DocsKey][] = [
      ["editing", "editing"],
      ["suggesting", "suggesting"],
      ["viewing", "viewing"],
    ];
    const allowed = modes.filter(([m]) => (m === "editing" ? this.canEdit() : m === "suggesting" ? o.role !== "view" : true));
    const unique = [...new Map(this.people.map((p) => [p.user, p])).values()];
    const threads = [...new Set(this.comments.filter((c) => !c.resolved).map((c) => c.thread))];
    return html`<header>
        <button class="back" aria-label=${t(this.localize, "back")} @click=${() => location.assign("/drive")}>${document.dir === "rtl" ? "›" : "‹"}</button>
        <div class="title">
          <input aria-label=${t(this.localize, "docName")} .value=${o.name} ?readonly=${!this.canEdit()} @change=${(e: Event) => this.rename((e.target as HTMLInputElement).value)} />
          <small>${[...o.path, this.saved ? t(this.localize, "saved") : t(this.localize, "saving")].join(" / ").replace(/ \/ ([^/]+)$/, " · $1")}</small>
        </div>
        ${o.label ? html`<span class="chip label">${o.label.toUpperCase()}</span>` : nothing}
        <span class="chip">${t(this.localize, stateKey)}</span>
        <span class="spacer"></span>
        <div class="people" aria-label=${t(this.localize, "hereNow")}>${unique.map((p) => html`<span title=${p.name ?? ""} .style=${`background: ${tone(p.name ?? p.user)}`}>${initials(p.name ?? "?")}</span>`)}</div>
        <select class="mode" aria-label=${t(this.localize, "mode")} .value=${this.mode} @change=${(e: Event) => ((this.mode = (e.target as HTMLSelectElement).value as Mode), this.view?.setProps({}))}>
          ${allowed.map(([m, key]) => html`<option value=${m} ?selected=${m === this.mode}>${t(this.localize, key)}</option>`)}
        </select>
        ${this.canEdit() && review?.state !== "open" ? html`<button class="button" @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("dialog.approval")?.showModal()}>${t(this.localize, "sendForApproval")}</button>` : nothing}
        ${this.canEdit() && review?.state === "open" ? html`<button class="button" @click=${this.withdraw}>${t(this.localize, "withdraw")}</button>` : nothing}
        ${this.canPublish ? html`<button class="button" ?disabled=${this.published} @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("dialog.publish")?.showModal()}>${this.published ? t(this.localize, "publishedMustRead") : t(this.localize, "publishMustRead")}</button>` : nothing}
        <button class="button primary" @click=${() => location.assign(`/drive?file=${encodeURIComponent(o.id)}&share=1`)}>${t(this.localize, "share")}</button>
      </header>
      <div class="tools" role="toolbar" aria-label=${t(this.localize, "format")}>
        <select aria-label=${t(this.localize, "style")} @change=${(e: Event) => this.block((e.target as HTMLSelectElement).value)}>
          <option value="p">${t(this.localize, "text")}</option>
          <option value="1">${t(this.localize, "heading1")}</option>
          <option value="2">${t(this.localize, "heading2")}</option>
        </select>
        <button aria-label=${t(this.localize, "bold")} aria-pressed=${this.isActive("strong") ? "true" : "false"} @click=${() => this.command(toggleMark(schema.marks.strong!))}><b>B</b></button>
        <button aria-label=${t(this.localize, "italic")} aria-pressed=${this.isActive("em") ? "true" : "false"} @click=${() => this.command(toggleMark(schema.marks.em!))}><i>I</i></button>
        <button @click=${this.insertTable}>${t(this.localize, "table")}</button>
        <button @click=${() => this.command(addRowAfter)} aria-label=${t(this.localize, "addRow")}>+${t(this.localize, "row")}</button>
        <button @click=${this.startComment}>${t(this.localize, "comment")}</button>
        <button aria-pressed=${this.mode === "suggesting" ? "true" : "false"} ?disabled=${o.role === "view"} @click=${() => ((this.mode = this.mode === "suggesting" ? (this.canEdit() ? "editing" : "suggesting") : "suggesting"), this.view?.setProps({}))}>${this.mode === "suggesting" ? t(this.localize, "trackOn") : t(this.localize, "trackOff")}</button>
        <span class="spacer"></span>
        <button @click=${() => this.view && downloadDocx(o.name, this.view.state.doc.toJSON())}>${t(this.localize, "downloadWord")}</button>
        <button @click=${() => printPdf(o.name, this.page)}>${t(this.localize, "pdf")}</button>
      </div>
      <div class="body">
        <div class="page"></div>
        <aside aria-label=${t(this.localize, "notes")}>
          ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
          ${this.draftComment
            ? html`<form class="card" @submit=${this.saveComment} aria-label=${t(this.localize, "newComment")}>
                <small>“${this.draftComment.quote}”</small>
                <textarea class="input" rows="3" required aria-label=${t(this.localize, "comment")}></textarea>
                <input class="input" type="email" placeholder=${t(this.localize, "assignTo")} aria-label=${t(this.localize, "assignTo")} />
                <div class="row"><button type="button" class="button" @click=${() => (this.draftComment = null)}>${t(this.localize, "cancel")}</button><button class="button primary">${t(this.localize, "comment")}</button></div>
              </form>`
            : nothing}
          ${this.list.map(
            (s) => html`<section class="card" aria-label=${t(this.localize, "suggestionBy", { who: s.author })}>
              <span class="who"><b>${s.author}</b> ${t(this.localize, "suggested")} · ${ago(s.at, lang)}</span>
              <p>${s.deleted && s.inserted ? t(this.localize, "replace", { old: s.deleted, new: s.inserted }) : s.inserted ? t(this.localize, "insert", { new: s.inserted }) : t(this.localize, "remove", { old: s.deleted })}</p>
              ${this.canEdit() ? html`<div class="row"><button class="button primary" @click=${() => this.decide(s, true)}>${t(this.localize, "accept")}</button><button class="button" @click=${() => this.decide(s, false)}>${t(this.localize, "reject")}</button></div>` : nothing}
            </section>`,
          )}
          ${threads.map((thread) => {
            const posts = this.comments.filter((c) => c.thread === thread);
            const first = posts[0]!;
            return html`<section class="card" aria-label=${t(this.localize, "commentBy", { who: first.author })}>
              <span class="who"><b>${first.author}</b> ${first.quote ? t(this.localize, "on", { what: first.quote }) : ""}</span>
              ${posts.map((p, i) => html`<p>${i ? html`<b>${p.author}</b>: ` : nothing}${p.body}</p>`)}
              <small>${posts.length > 1 ? t(this.localize, "replies", { count: posts.length - 1 }) : ""}${first.assignedTo ? html` · <span class="assigned">${t(this.localize, "assignedTo", { who: first.assignedTo })}</span>` : nothing}</small>
              ${o.role !== "view"
                ? html`<form class="row" @submit=${(e: Event) => (e.preventDefault(), this.reply(thread, e.target as HTMLFormElement))}>
                    <input class="input" aria-label=${t(this.localize, "reply")} placeholder=${t(this.localize, "reply")} />
                    <button type="button" class="button" @click=${() => this.resolve(thread)}>${t(this.localize, "resolve")}</button>
                  </form>`
                : nothing}
            </section>`;
          })}
        </aside>
      </div>
      <dialog class="publish" aria-label=${t(this.localize, "publishMustRead")}>
        <form @submit=${this.publish}>
          <h2>${t(this.localize, "publishMustRead")}</h2>
          <p>${t(this.localize, "publishLead")}</p>
          <label>${t(this.localize, "publishGroups")}<input class="input" name="groups" placeholder=${t(this.localize, "everyone")} /></label>
          <label>${t(this.localize, "publishNote")}<input class="input" name="note" /></label>
          ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
          <div class="row"><button type="button" class="button" @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("dialog.publish")?.close()}>${t(this.localize, "cancel")}</button><button class="button primary">${t(this.localize, "publish")}</button></div>
        </form>
      </dialog>
      <dialog class="approval" aria-label=${t(this.localize, "sendForApproval")}>
        <form @submit=${this.sendForApproval}>
          <h2>${t(this.localize, "sendForApproval")}</h2>
          <p>${t(this.localize, "routeLead")}</p>
          <div class="steps">
            ${this.steps.map(
              (s, i) => html`<div class="step">
                <input class="input" type="email" placeholder=${t(this.localize, "reviewer")} aria-label=${t(this.localize, "reviewerN", { n: i + 1 })} .value=${s.email} @input=${(e: Event) => (this.steps = this.steps.map((x, j) => (j === i ? { ...x, email: (e.target as HTMLInputElement).value } : x)))} />
                <input class="input" placeholder=${t(this.localize, "stepTitle")} aria-label=${t(this.localize, "stepTitleN", { n: i + 1 })} .value=${s.title} @input=${(e: Event) => (this.steps = this.steps.map((x, j) => (j === i ? { ...x, title: (e.target as HTMLInputElement).value } : x)))} />
                <input class="input" type="datetime-local" aria-label=${t(this.localize, "dueN", { n: i + 1 })} .value=${s.due} @input=${(e: Event) => (this.steps = this.steps.map((x, j) => (j === i ? { ...x, due: (e.target as HTMLInputElement).value } : x)))} />
                <select class="input" aria-label=${t(this.localize, "kindN", { n: i + 1 })} .value=${s.kind} @change=${(e: Event) => (this.steps = this.steps.map((x, j) => (j === i ? { ...x, kind: (e.target as HTMLSelectElement).value as "approve" | "sign" } : x)))}>
                  <option value="approve" ?selected=${s.kind === "approve"}>${t(this.localize, "approves")}</option>
                  <option value="sign" ?selected=${s.kind === "sign"}>${t(this.localize, "signs")}</option>
                </select>
              </div>`,
            )}
          </div>
          <button type="button" class="button" @click=${() => (this.steps = [...this.steps, { email: "", title: "", due: "", kind: "approve" }])}>${t(this.localize, "addStep")}</button>
          <label>${t(this.localize, "caseRef")}<input class="input" .value=${this.caseRef} placeholder="2026/EM/0114" @input=${(e: Event) => (this.caseRef = (e.target as HTMLInputElement).value)} /></label>
          ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
          <div class="row"><button type="button" class="button" @click=${() => this.renderRoot.querySelector<HTMLDialogElement>("dialog.approval")?.close()}>${t(this.localize, "cancel")}</button><button class="button primary">${t(this.localize, "send")}</button></div>
        </form>
      </dialog>`;
  }
}
