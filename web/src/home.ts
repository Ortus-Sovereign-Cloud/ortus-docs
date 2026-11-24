// Docs' home: a new document, and the reviews waiting for one. The
// documents themselves are in Drive.
import { ApiError, LitElement, LocalizeController, call, controls, css, customElement, html, nothing, state } from "ortus-kit";
import { t } from "./i18n.js";
import { newFromDocx } from "./word.js";

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
      label input {
        position: absolute;
        inline-size: 1px;
        block-size: 1px;
        opacity: 0;
      }
      label.button {
        position: relative;
        cursor: pointer;
      }
      .error {
        color: var(--ortus-red);
      }
      /* A phone: the buttons wrap under the title. */
      @media (max-width: 720px) {
        :host {
          padding-block: 16px;
          padding-inline: 12px;
        }
        .top {
          flex-wrap: wrap;
          gap: 8px;
        }
        h1 {
          flex-basis: 100%;
        }
        .button {
          min-block-size: 44px;
          margin: 0;
        }
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

  private async importWord(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    this.error = "";
    try {
      const path = await newFromDocx(file);
      history.pushState(null, "", path);
      window.dispatchEvent(new PopStateEvent("popstate"));
    } catch {
      this.error = t(this.localize, "notWord");
    }
  }

  override render() {
    return html`<div class="top">
        <h1>${t(this.localize, "docs")}</h1>
        <a class="button" href="/drive">${t(this.localize, "inDrive")}</a>
        <label class="button">${t(this.localize, "importWord")}<input type="file" accept=".docx,application/vnd.openxmlformats-officedocument.wordprocessingml.document" @change=${this.importWord} /></label>
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
