// The docs client: the element the shell shows at /docs — a document
// (?doc=), a review (?review=) or the home — by address.
import { LitElement, css, customElement, html, state } from "ortus-kit";
import "./editor.js";
import "./home.js";
import "./review.js";
import { ready } from "./i18n.js";

await ready;

function place(): { doc: string | null; review: string | null } {
  const p = new URLSearchParams(location.search);
  return { doc: p.get("doc"), review: p.get("review") };
}

@customElement("ortus-docs-app")
export class OrtusDocsApp extends LitElement {
  static override styles = css`
    :host {
      display: block;
    }
  `;

  @state() private at = place();

  private readonly onPop = (): void => {
    this.at = place();
  };

  override connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener("popstate", this.onPop);
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener("popstate", this.onPop);
  }

  override render() {
    // A new element for each address: each loads what it names.
    if (this.at.doc) return html`${[this.at.doc].map(() => html`<ortus-docs-editor></ortus-docs-editor>`)}`;
    if (this.at.review) return html`${[this.at.review].map(() => html`<ortus-docs-review></ortus-docs-review>`)}`;
    return html`<ortus-docs-home></ortus-docs-home>`;
  }
}
