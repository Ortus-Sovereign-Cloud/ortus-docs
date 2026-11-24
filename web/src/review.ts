// Reviewing (board WebReview): the document as it was sent, with what
// changed since the version the last review saw marked; one's turn in
// the route to approve or send back; the route's steps.
import { ApiError, LitElement, LocalizeController, call, controls, css, customElement, html, nothing, state, type TemplateResult } from "ortus-kit";
import { t } from "./i18n.js";

interface Step {
  n: number;
  user: string;
  name: string;
  title: string;
  dueAt: number | null;
  state: "current" | "waiting" | "approved" | "signed" | "returned" | "cancelled" | "skipped";
  comment: string;
  decidedAt: number | null;
  kind: "approve" | "sign";
}

interface Review {
  id: string;
  doc: string;
  by: string;
  state: string;
  version: number;
  previousVersion: number | null;
  case: string | null;
  signed: boolean;
  record: string | null;
  at: number;
  steps: Step[];
}

interface PMNode {
  type: string;
  text?: string;
  attrs?: { level?: number };
  marks?: { type: string }[];
  content?: PMNode[];
}

function lang(): string {
  return document.documentElement.lang || "en";
}

function when(ms: number): string {
  return new Intl.DateTimeFormat(lang(), { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date(ms));
}

function hm(ms: number): string {
  return new Intl.DateTimeFormat(lang(), { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date(ms));
}

@customElement("ortus-docs-review")
export class OrtusDocsReview extends LitElement {
  static override styles = [
    controls,
    css`
      :host {
        display: block;
        min-block-size: var(--ortus-screen, 100vh);
        color: var(--ortus-text);
      }
      header {
        display: flex;
        align-items: center;
        gap: 12px;
        padding-block: 18px;
        padding-inline: 32px;
        border-block-end: 1px solid var(--ortus-line);
      }
      header h1 {
        margin: 0;
        font-size: 16px;
        font-weight: 500;
        flex: 1;
      }
      header a {
        color: var(--ortus-text);
        font-weight: 600;
        text-decoration: none;
      }
      .label {
        padding-block: 3px;
        padding-inline: 8px;
        border-radius: 6px;
        font-family: var(--ortus-mono, monospace);
        font-size: 12px;
        background: color-mix(in srgb, var(--ortus-orange) 18%, transparent);
        color: var(--ortus-orange);
      }
      .body {
        display: grid;
        grid-template-columns: minmax(0, 700px) 420px;
        gap: 24px;
        padding-block: 24px;
        padding-inline: 112px 24px;
        align-items: start;
      }
      .page {
        min-block-size: 70vh;
        padding-block: 56px;
        padding-inline: 56px;
        border-radius: 6px;
        background: #fafaf8;
        color: #1b1d24;
        font-family: "Source Serif 4", Georgia, serif;
        font-size: 17px;
        line-height: 1.6;
      }
      .page h1 {
        font-size: 30px;
        margin-block: 0 24px;
      }
      .page h2 {
        font-size: 20px;
        margin-block: 24px 12px;
      }
      .page p {
        margin-block: 0 14px;
      }
      .page table {
        inline-size: 100%;
        border-collapse: collapse;
        font-family: var(--ortus-font);
        font-size: 15px;
      }
      .page td,
      .page th {
        padding-block: 10px;
        padding-inline: 12px;
        border-block-end: 1px solid #d9dce3;
        text-align: start;
        font-weight: 400;
      }
      .edited {
        margin-block: -24px 24px;
        font-family: var(--ortus-font);
        font-size: 13px;
        color: #8a5a00;
      }
      .changed {
        background: #d9f5e6;
      }
      .page footer {
        margin-block-start: 32px;
        font-family: var(--ortus-font);
        font-size: 13px;
        color: #6b7080;
      }
      aside {
        display: grid;
        gap: 16px;
      }
      .card {
        display: grid;
        gap: 12px;
        padding: 24px;
        border-radius: 18px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
      }
      .card.turn {
        border-color: var(--ortus-line-strong);
      }
      .turn small:first-child {
        color: var(--ortus-accent);
        font-weight: 600;
      }
      .card h2 {
        margin: 0;
        font-size: 20px;
      }
      .card p,
      .card small {
        margin: 0;
        color: var(--ortus-text-3);
        font-size: 14px;
      }
      .row {
        display: flex;
        gap: 10px;
      }
      .row .primary {
        flex: 1;
      }
      dl {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: 10px 20px;
        margin: 0;
        font-size: 14px;
      }
      dt {
        color: var(--ortus-text-3);
      }
      dd {
        margin: 0;
      }
      ol {
        margin: 0;
        padding: 0;
        list-style: none;
        display: grid;
        gap: 16px;
      }
      ol li {
        display: flex;
        gap: 14px;
        align-items: center;
      }
      .n {
        display: grid;
        place-items: center;
        inline-size: 32px;
        block-size: 32px;
        border-radius: 8px;
        background: var(--ortus-surface-2);
        font-weight: 700;
      }
      .n.current {
        background: var(--ortus-accent);
        color: #0a0b10;
      }
      .n.approved {
        color: var(--ortus-teal);
      }
      .n.returned {
        color: var(--ortus-red);
      }
      ol strong {
        display: block;
      }
      ol small {
        color: var(--ortus-text-3);
      }
      textarea {
        font: inherit;
      }
      .boxes {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 12px;
        margin-block-start: 28px;
        font-family: var(--ortus-font);
      }
      .box {
        display: grid;
        gap: 4px;
        padding: 14px;
        border-radius: 10px;
        border: 1px dashed #b9bdc8;
        font-size: 13px;
        color: #4b5060;
      }
      .box.done {
        border: 1px solid #8fd6b7;
        background: #eefaf4;
      }
      .box strong {
        color: #1b1d24;
        font-size: 14px;
      }
      .box em {
        font-style: normal;
        font-weight: 600;
      }
      .box.done em {
        color: #1d7a55;
      }
      .case {
        color: var(--ortus-text-3);
        font-size: 13px;
      }
      .case b {
        font-family: var(--ortus-mono, monospace);
        font-weight: 500;
        color: var(--ortus-text-2);
      }
      label.confirm {
        display: grid;
        gap: 6px;
        font-size: 13px;
        color: var(--ortus-text-2);
      }
      .error {
        color: var(--ortus-red);
        margin: 0;
      }
      .phone {
        display: none;
      }
      /* A phone: one pane at a time. The document first; the decision
         and the route behind a button, with a way back. */
      @media (max-width: 720px) {
        header {
          flex-wrap: wrap;
          gap: 8px;
          padding-block: 8px;
          padding-inline: 4px 12px;
        }
        header h1 {
          flex: 1 1 calc(100% - 60px);
          min-inline-size: 0;
          overflow-wrap: anywhere;
        }
        .phone {
          display: inline-flex;
        }
        .up {
          align-items: center;
          justify-content: center;
          inline-size: 44px;
          block-size: 44px;
          border: 0;
          background: none;
          color: var(--ortus-text);
          font-size: 18px;
          cursor: pointer;
        }
        .button {
          min-block-size: 44px;
        }
        .body {
          grid-template-columns: minmax(0, 1fr);
          gap: 12px;
          padding-block: 12px;
          padding-inline: 8px;
        }
        .page {
          padding-block: 24px;
          padding-inline: 18px;
        }
        .tablebox {
          overflow-x: auto;
        }
        .boxes {
          grid-template-columns: minmax(0, 1fr);
        }
        aside {
          display: none;
        }
        .body.side .page {
          display: none;
        }
        .body.side aside {
          display: grid;
        }
        .side-back {
          justify-self: start;
          gap: 6px;
        }
        .card {
          padding: 16px;
        }
        .row {
          flex-wrap: wrap;
        }
      }
    `,
  ];

  private readonly localize = new LocalizeController(this);
  @state() private review: Review | null = null;
  @state() private doc: PMNode | null = null;
  @state() private name = "";
  @state() private label: string | null = null;
  @state() private earlier: string[] = [];
  @state() private edited = false;
  @state() private me = "";
  @state() private error = "";
  @state() private busy = false;
  @state() private confirming = false;
  /** On a phone: the decision and the route show instead of the document. */
  @state() private side = false;

  override connectedCallback(): void {
    super.connectedCallback();
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      const id = new URLSearchParams(location.search).get("review") ?? "";
      const [review, who] = await Promise.all([call<Review>("GET", `/docs/reviews/${encodeURIComponent(id)}`), call<{ id: string }>("GET", "/me")]);
      this.review = review;
      this.me = who.id;
      // The document as it was sent, not as it is now.
      const sent = await call<{ before: string | null; doc: PMNode | null; name: string; label: string | null; edited: boolean }>("GET", `/docs/reviews/${encodeURIComponent(review.id)}/document`);
      this.doc = sent.doc;
      this.name = sent.name;
      this.label = sent.label;
      this.edited = sent.edited;
      this.earlier = review.previousVersion && sent.before !== null ? sent.before.split("\n").map((l) => l.trim()) : [];
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
    }
  }

  private textOf(node: PMNode): string {
    return (node.text ?? "") + (node.content ?? []).map((c) => this.textOf(c)).join("");
  }

  /** A block is marked when its text was not in the earlier version. */
  private isNew(node: PMNode): boolean {
    return !!this.review?.previousVersion && !this.earlier.includes(this.textOf(node).trim());
  }

  private inline(nodes: PMNode[] = []): (TemplateResult | string)[] {
    // The same text the PDF sets: every text inside, as it stands
    // (a route is only sent with no suggestions open).
    return nodes.flatMap((n) => {
      if (n.type === "hard_break") return ["\n"];
      if (n.type !== "text") return this.inline(n.content);
      let out: TemplateResult | string = n.text ?? "";
      if (n.marks?.some((m) => m.type === "strong")) out = html`<strong>${out}</strong>`;
      if (n.marks?.some((m) => m.type === "em")) out = html`<em>${out}</em>`;
      return out;
    });
  }

  private block(node: PMNode): TemplateResult | typeof nothing {
    const cls = this.isNew(node) ? "changed" : "";
    switch (node.type) {
      case "heading":
        return node.attrs?.level === 1 ? html`<h1 class=${cls}>${this.inline(node.content)}</h1>` : html`<h2 class=${cls}>${this.inline(node.content)}</h2>`;
      case "paragraph":
        return html`<p class=${cls}>${this.inline(node.content)}</p>`;
      case "bullet_list":
        return html`<ul>${(node.content ?? []).map((li) => html`<li>${(li.content ?? []).map((b) => this.block(b))}</li>`)}</ul>`;
      case "ordered_list":
        return html`<ol>${(node.content ?? []).map((li) => html`<li>${(li.content ?? []).map((b) => this.block(b))}</li>`)}</ol>`;
      case "blockquote":
        return html`<blockquote>${(node.content ?? []).map((b) => this.block(b))}</blockquote>`;
      case "table":
        return html`<div class="tablebox"><table>
          ${(node.content ?? []).map((row) => html`<tr>${(row.content ?? []).map((cell) => (cell.type === "table_header" ? html`<th>${this.textOf(cell)}</th>` : html`<td>${this.textOf(cell)}</td>`))}</tr>`)}
        </table></div>`;
      default:
        return nothing;
    }
  }

  private async decide(approve: boolean): Promise<void> {
    const r = this.review;
    if (!r) return;
    const comment = this.renderRoot.querySelector<HTMLTextAreaElement>("textarea")?.value ?? "";
    const password = this.renderRoot.querySelector<HTMLInputElement>("input.password")?.value || undefined;
    this.busy = true;
    this.error = "";
    try {
      this.review = await call<Review>("POST", `/docs/reviews/${encodeURIComponent(r.id)}/decide`, { approve, comment, password });
      this.confirming = false;
    } catch (e) {
      // Signing asks who it is when the sign-in is old.
      if (e instanceof ApiError && e.code === "confirm") this.confirming = true;
      this.error = e instanceof ApiError ? e.message : String(e);
    } finally {
      this.busy = false;
    }
  }

  override render() {
    const r = this.review;
    if (!r) return html`${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}`;
    const mine = r.steps.find((s) => s.state === "current" && s.user === this.me);
    const next = mine ? r.steps.find((s) => s.n === mine.n + 1) : undefined;
    const changed = (this.doc?.content ?? []).filter((b) => b.type !== "table" && this.isNew(b)).map((b) => this.textOf(b));
    return html`<header>
        <button class="phone up" aria-label=${t(this.localize, "back")} @click=${() => location.assign("/docs")}>${document.dir === "rtl" ? "›" : "‹"}</button>
        <h1><a href="/docs">${t(this.localize, "approvals")}</a> / ${this.name} · v${r.version}</h1>
        ${r.case ? html`<span class="case">${t(this.localize, "caseLabel")} <b>${r.case}</b></span>` : nothing}
        ${this.label ? html`<span class="label">${this.label.toUpperCase()}</span>` : nothing}
        <button class="button phone ${mine ? "primary" : ""}" aria-expanded=${this.side ? "true" : "false"} @click=${() => (this.side = !this.side)}>${mine ? t(this.localize, "yourTurn") : t(this.localize, "approvalRoute")}</button>
      </header>
      <div class="body ${this.side ? "side" : ""}">
        <article class="page" aria-label=${this.name}>
          ${this.edited ? html`<p class="edited" role="note">${t(this.localize, "editedSince", { n: r.version })}</p>` : nothing}
          ${(this.doc?.content ?? []).map((b) => this.block(b))}
          ${changed.length ? html`<footer>${t(this.localize, "markedChanged", { count: changed.length })}</footer>` : nothing}
          ${r.signed
            ? html`<div class="boxes">
                ${r.steps.map(
                  (s) => html`<div class="box ${s.state === "signed" || s.state === "approved" ? "done" : ""}">
                    <strong>${s.name}</strong><span>${s.title}</span>
                    <em>${s.state === "signed" ? t(this.localize, "signedAt", { time: when(s.decidedAt!) }) : s.state === "approved" ? t(this.localize, "approvedAt", { time: when(s.decidedAt!) }) : t(this.localize, "waitingWord")}</em>
                  </div>`,
                )}
              </div>`
            : nothing}
        </article>
        <aside>
          <button class="button phone side-back" @click=${() => (this.side = false)}><span aria-hidden="true">${document.dir === "rtl" ? "›" : "‹"}</span>${t(this.localize, "back")}</button>
          ${mine
            ? html`<section class="card turn" aria-label=${t(this.localize, "yourTurn")}>
                <small>${t(this.localize, "yourTurnStep", { n: mine.n, count: r.steps.length })}</small>
                <h2>${t(this.localize, "asksYou", { who: r.by.split(" ")[0] ?? r.by, what: mine.title || t(this.localize, "toReview") })}</h2>
                <p>${t(this.localize, "sentAt", { time: hm(r.at) })}${next ? ` · ${t(this.localize, "nextAfterYou", { who: next.name })}` : ""}${mine.dueAt ? ` · ${t(this.localize, "byTime", { time: hm(mine.dueAt) })}` : ""}</p>
                <textarea class="input" rows="3" placeholder=${t(this.localize, "commentOptional")} aria-label=${t(this.localize, "commentOptional")}></textarea>
                ${this.confirming
                  ? html`<label class="confirm">${t(this.localize, "confirmPassword")}<input class="input password" type="password" autocomplete="current-password" /></label>
                      <small>${t(this.localize, "orSignInAgain")} <a href="/sign-in">${t(this.localize, "signInAgain")}</a></small>`
                  : nothing}
                ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
                <div class="row">
                  <button class="button primary" ?disabled=${this.busy} @click=${() => this.decide(true)}>${mine.kind === "sign" ? t(this.localize, "sign") : t(this.localize, "approve")}</button>
                  <button class="button" ?disabled=${this.busy} @click=${() => this.decide(false)}>${t(this.localize, "sendBack")}</button>
                </div>
                ${mine.kind === "sign"
                  ? html`<small>${t(this.localize, "signNote")}</small>
                      <a href=${`/api/v1/docs/reviews/${encodeURIComponent(r.id)}/pdf`} target="_blank" rel="noopener">${t(this.localize, "openPdf")}</a>`
                  : nothing}
              </section>`
            : html`<section class="card" role="status"><p>${r.state === "approved" ? (r.signed ? t(this.localize, "allSigned") : t(this.localize, "allApproved")) : r.state === "returned" ? t(this.localize, "wasReturned") : t(this.localize, "notYourTurn")}</p>${r.record ? html`<a class="button" href=${r.record}>${t(this.localize, "openRecord")}</a>` : nothing}</section>`}
          ${r.previousVersion
            ? html`<section class="card" aria-label=${t(this.localize, "changedSince", { n: r.previousVersion })}>
                <strong>${t(this.localize, "changedSince", { n: r.previousVersion })}</strong>
                ${changed.length ? html`<dl>${changed.map((c) => html`<dt>·</dt><dd>${c}</dd>`)}</dl>` : html`<p>${t(this.localize, "noChanges")}</p>`}
              </section>`
            : nothing}
          <section class="card" aria-label=${t(this.localize, "route")}>
            <h2>${t(this.localize, "approvalRoute")}</h2>
            <ol>
              ${r.steps.map(
                (s) => html`<li>
                  <span class="n ${s.state}" aria-hidden="true">${s.state === "approved" || s.state === "signed" ? "✓" : s.state === "returned" ? "↩" : s.n}</span>
                  <span><strong>${s.title || t(this.localize, "toReview")}</strong><small>${s.user === this.me ? t(this.localize, "you") : s.name}${s.decidedAt ? ` · ${hm(s.decidedAt)}` : s.state === "current" ? ` · ${t(this.localize, "now")}` : ""}${s.comment ? ` · “${s.comment}”` : ""}</small></span>
                </li>`,
              )}
            ${r.signed
                ? html`<li>
                    <span class="n ${r.record ? "approved" : ""}" aria-hidden="true">${r.record ? "✓" : r.steps.length + 1}</span>
                    <span><strong>${t(this.localize, "fileStep")}</strong><small>${r.case ? t(this.localize, "fileTo", { case: r.case }) : t(this.localize, "fileAuto")}</small></span>
                  </li>`
                : nothing}
            </ol>
          </section>
        </aside>
      </div>`;
  }
}
