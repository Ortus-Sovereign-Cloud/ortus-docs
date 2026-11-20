// Docs' home: a new document, and the reviews waiting for one. The
// documents themselves are in Drive.
import { ApiError, LitElement, LocalizeController, call, controls, css, customElement, html, nothing, state } from "ortus-kit";
import { t } from "./i18n.js";

interface Waiting {
  id: string;
  name: string;
  by: string;
  at: number;
}

@customElement("ortus-docs-home")
export class OrtusDocsHome extends LitElement {
  static override styles = [
    controls,
    css`
      :host {
        display: block;
        padding-block: 32px;
        padding-inline: 32px;
        color: var(--ortus-text);
      }
      .top {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-block-end: 24px;
      }
      h1 {
        margin: 0;
        font-size: 24px;
        flex: 1;
      }
      h2 {
        font-size: 15px;
        color: var(--ortus-text-2);
        font-weight: 500;
      }
      ul {
        margin: 0;
        padding: 0;
        list-style: none;
        display: grid;
        gap: 10px;
        max-inline-size: 720px;
      }
      a.item {
        display: grid;
        gap: 4px;
        padding: 16px;
        border-radius: 14px;
        border: 1px solid var(--ortus-line);
        background: var(--ortus-surface);
        color: inherit;
        text-decoration: none;
      }
      a.item small,
      p {
        color: var(--ortus-text-3);
      }
      .error {
        color: var(--ortus-red);
      }
    `,
  ];

  private readonly localize = new LocalizeController(this);
  @state() private waiting: Waiting[] = [];
  @state() private error = "";

  override connectedCallback(): void {
    super.connectedCallback();
    void call<{ waiting: Waiting[] }>("GET", "/docs/reviews/mine")
      .then((r) => (this.waiting = r.waiting))
      .catch(() => undefined);
  }

  private async create(): Promise<void> {
    try {
      const made = await call<{ id: string; path: string }>("POST", "/docs/documents", { name: t(this.localize, "untitled") });
      history.pushState(null, "", made.path);
      window.dispatchEvent(new PopStateEvent("popstate"));
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e);
    }
  }

  override render() {
    return html`<div class="top">
        <h1>${t(this.localize, "docs")}</h1>
        <a class="button" href="/drive">${t(this.localize, "inDrive")}</a>
        <button class="button primary" @click=${this.create}>${t(this.localize, "newDocument")}</button>
      </div>
      ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
      <h2>${t(this.localize, "waitingForYou")}</h2>
      ${this.waiting.length
        ? html`<ul aria-label=${t(this.localize, "waitingForYou")}>
            ${this.waiting.map((w) => html`<li><a class="item" href="/docs?review=${encodeURIComponent(w.id)}" @click=${(e: Event) => (e.preventDefault(), history.pushState(null, "", `/docs?review=${encodeURIComponent(w.id)}`), window.dispatchEvent(new PopStateEvent("popstate")))}><strong>${w.name}</strong><small>${t(this.localize, "sentBy", { who: w.by })}</small></a></li>`)}
          </ul>`
        : html`<p role="status">${t(this.localize, "nothingWaiting")}</p>`}`;
  }
}
