// Keeps a Yjs document in step with the server: local changes go up by
// POST, everyone's come down as server-sent events, presence (cursors,
// who is here) goes both ways, and now and then the whole document goes
// up as a checkpoint that folds the loose updates in.
import * as Y from "yjs";
import { Awareness, applyAwarenessUpdate, encodeAwarenessUpdate } from "y-protocols/awareness";

const REMOTE = "remote";

function b64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

function unb64(text: string): Uint8Array {
  const s = atob(text);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

export interface Opened {
  id: string;
  name: string;
  path: string[];
  label: string | null;
  role: "view" | "comment" | "edit" | "owner";
  base: string | null;
  updates: string[];
  seq: number;
  me: { id: string; name: string };
  review: unknown;
}

export interface Here {
  client: string;
  user: string;
  name: string | null;
}

export class Sync {
  readonly doc = new Y.Doc();
  readonly awareness = new Awareness(this.doc);
  readonly client = Math.random().toString(36).slice(2, 14);
  seq = 0;
  /** Changes not yet confirmed by the server. */
  pending = 0;
  onPeople: (people: Here[]) => void = () => undefined;
  onSaved: (saved: boolean) => void = () => undefined;
  onGone: () => void = () => undefined;
  private events: EventSource | null = null;
  private queue: Uint8Array[] = [];
  private sending = false;
  private sinceCheckpoint = 0;
  private presenceTimer = 0;
  private checkpointTimer = 0;
  private closed = false;

  constructor(
    readonly id: string,
    private readonly canWrite: boolean,
    private readonly structure: () => unknown,
  ) {}

  /** Starts from what the server has, then listens. */
  start(opened: Opened): void {
    if (opened.base) Y.applyUpdate(this.doc, unb64(opened.base), REMOTE);
    for (const u of opened.updates) Y.applyUpdate(this.doc, unb64(u), REMOTE);
    this.seq = opened.seq;
    this.sinceCheckpoint = opened.updates.length;
    this.doc.on("update", (update: Uint8Array, origin: unknown) => {
      if (origin === REMOTE || !this.canWrite) return;
      this.queue.push(update);
      void this.flush();
    });
    this.awareness.on("update", ({ added, updated, removed }: { added: number[]; updated: number[]; removed: number[] }, origin: unknown) => {
      if (origin === REMOTE) return;
      const changed = [...added, ...updated, ...removed];
      if (changed.includes(this.doc.clientID)) this.sendPresence();
    });
    this.listen();
    this.presenceTimer = window.setInterval(() => this.sendPresence(), 10_000);
    this.checkpointTimer = window.setInterval(() => void this.checkpoint(), 30_000);
  }

  private listen(): void {
    if (this.closed) return;
    this.events = new EventSource(`/api/v1/docs/documents/${encodeURIComponent(this.id)}/events?since=${this.seq}`);
    this.events.addEventListener("update", (e) => {
      const m = JSON.parse((e as MessageEvent).data) as { seq: number; data: string; client: string };
      this.seq = Math.max(this.seq, m.seq);
      this.sinceCheckpoint++;
      if (m.client !== this.client) Y.applyUpdate(this.doc, unb64(m.data), REMOTE);
    });
    this.events.addEventListener("presence", (e) => {
      const list = JSON.parse((e as MessageEvent).data) as (Here & { state: string })[];
      for (const p of list) {
        if (p.client !== this.client) {
          try {
            applyAwarenessUpdate(this.awareness, unb64(p.state), REMOTE);
          } catch {
            /* a broken state is ignored */
          }
        }
      }
      this.onPeople(list.map(({ client, user, name }) => ({ client, user, name })));
    });
    this.events.addEventListener("reload", () => this.onGone());
    // The server ends a stream after a while: come back from where we were.
    this.events.onerror = () => {
      this.events?.close();
      window.setTimeout(() => this.listen(), 1000);
    };
  }

  private async flush(): Promise<void> {
    if (this.sending || !this.queue.length) return;
    this.sending = true;
    this.onSaved(false);
    const update = Y.mergeUpdates(this.queue.splice(0));
    this.pending++;
    try {
      const r = await fetch(`/api/v1/docs/documents/${encodeURIComponent(this.id)}/updates?client=${this.client}`, { method: "POST", headers: { "X-Requested-With": "ortus", "Content-Type": "application/octet-stream" }, body: new Blob([update as BlobPart]) });
      if (r.status === 409) {
        await this.checkpoint();
        this.queue.unshift(update);
      } else if (!r.ok) {
        this.queue.unshift(update);
        await new Promise((done) => window.setTimeout(done, 2000));
      }
    } catch {
      this.queue.unshift(update);
      await new Promise((done) => window.setTimeout(done, 2000));
    } finally {
      this.pending--;
      this.sending = false;
    }
    if (this.queue.length) void this.flush();
    else this.onSaved(true);
  }

  private sendPresence(): void {
    if (this.closed) return;
    const state = b64(encodeAwarenessUpdate(this.awareness, [this.doc.clientID]));
    void fetch(`/api/v1/docs/documents/${encodeURIComponent(this.id)}/presence`, { method: "POST", headers: { "X-Requested-With": "ortus", "Content-Type": "application/json" }, body: JSON.stringify({ client: this.client, state }) }).catch(() => undefined);
  }

  /** The whole document up to what has been seen, with its structure. */
  async checkpoint(): Promise<void> {
    if (!this.canWrite || this.sending || this.queue.length || this.sinceCheckpoint === 0 || this.seq === 0) return;
    const seq = this.seq;
    const r = await fetch(`/api/v1/docs/documents/${encodeURIComponent(this.id)}/checkpoint`, {
      method: "POST",
      headers: { "X-Requested-With": "ortus", "Content-Type": "application/json" },
      body: JSON.stringify({ seq, state: b64(Y.encodeStateAsUpdate(this.doc)), doc: this.structure() }),
    }).catch(() => null);
    if (r?.ok) this.sinceCheckpoint = 0;
  }

  /** Everything sent, then a checkpoint (before sending for approval). */
  async settle(): Promise<void> {
    for (let i = 0; i < 50 && (this.queue.length || this.sending); i++) await new Promise((done) => window.setTimeout(done, 100));
    // Our own updates come back with their numbers: wait for them a moment.
    await new Promise((done) => window.setTimeout(done, 700));
    this.sinceCheckpoint = Math.max(this.sinceCheckpoint, 1);
    await this.checkpoint();
  }

  close(): void {
    this.closed = true;
    window.clearInterval(this.presenceTimer);
    window.clearInterval(this.checkpointTimer);
    this.events?.close();
    void this.checkpoint();
  }
}
