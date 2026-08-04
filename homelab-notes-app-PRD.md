# Homelab Notes — Product Requirements Document (v0.1 Draft)

*A self-hosted, cross-platform, markdown-first notes app — "your own Obsidian" — with native sync replacing Syncthing.*

---

## 1. Problem Statement

Today's setup: Obsidian (desktop + mobile) + Syncthing running in a homelab VM (`pve-II`), because Android has no first-class SMB/NFS client and can't talk to the TrueNAS share directly. This works, but:

- Syncthing is a generic file-sync tool, not note-aware — no real conflict resolution beyond "keep both files," no note metadata, no selective/partial sync of a large vault.
- It's another moving part (a whole VM) whose only job is to move `.md` files around.
- No web access to notes without exposing the vault some other way.
- No control over the actual editing experience, search, or future features (tags, backlinks, etc.) beyond what Obsidian's plugin ecosystem allows.

**Goal:** Replace Obsidian + Syncthing with a self-built system: native apps (Android, iOS, desktop, web) that edit plain markdown files and sync directly against a small server running in the homelab — no third-party sync layer, no cloud dependency, full control of the format and the protocol.

---

## 2. Guiding Principles

1. **Server is the source of truth; devices cache.** The homelab server holds the one canonical copy of the vault. Clients keep a local cache of recently/frequently opened notes so reads (and short offline edit sessions) work without a live connection, but the cache is not an independent copy of record — it reconciles back to the server, not the other way around.
2. **Plain markdown on disk, always.** No proprietary database format for note content. A note is a `.md` file with YAML frontmatter, full stop. This is non-negotiable — it's the property that made Obsidian trustworthy in the first place, and it's what keeps this project escapable (any future tool can read the vault).
3. **Self-hosted only.** No cloud relay, no third-party accounts. The sync server runs on your Proxmox cluster.
4. **Small v1.** Ruthlessly cut scope. No graph view, no plugin system, no themes marketplace. Get "write notes, sync notes, search notes" rock solid first.
5. **One codebase for client logic**, not five separate apps to maintain in parallel.

---

## 3. V1 Scope

### In scope
- Folder/file tree (vault browser), create/rename/move/delete notes and folders
- Markdown editor: live-preview or source mode (pick one for v1 — recommend a single hybrid editor, see §7), YAML frontmatter support
- Full-text search across the vault
- Wikilink-style `[[note name]]` linking and basic backlinks list (no graph *visualization*, just a "linked mentions" panel — this is cheap to build once you're parsing links for anything, and it's high-value)
- Tags (`#tag` inline or frontmatter `tags:`) with a tag browser
- Sync: multi-device, offline-capable, conflict-safe
- Attachments (images, PDFs) stored alongside notes and synced
- Basic settings: vault location, theme (light/dark), font size

### Explicitly out of scope for v1
- Graph view
- Plugin/extension system of any kind (community or API)
- Multiple vaults in one app instance (support one vault per app install initially)
- Real-time collaborative multi-cursor editing (sync is "eventually consistent across devices," not Google-Docs-style live co-editing)
- Mobile widgets, share-sheet capture, Apple Watch, etc.
- Encryption at rest on the server (can be a fast-follow — see §10)

---

## 4. Stack Decision

This is the question you asked me to answer first, so here it is up front, with the reasoning shown.

### 4.1 The real fork in the road

You framed it as "React Native or Rust," but those aren't actually alternatives to each other — they solve different halves of the problem:

- **React Native** is a UI framework (client side).
- **Rust** is a systems language, better thought of as a candidate for the **sync engine / backend**, or as the core of a cross-platform engine that multiple UI layers bind to.

So the real decision has two independent parts:

1. **What renders the UI on Android / iOS / Desktop / Web?**
2. **What powers the sync protocol and conflict resolution, and where does that logic live?**

### 4.2 Option A — Flutter (client) + Rust (sync core)

| | |
|---|---|
| **Client** | Flutter — single Dart codebase compiles to true native Android, iOS, Windows, macOS, Linux, and web (via CanvasKit/Wasm) |
| **Sync core** | Rust, compiled to a shared library (`.so`/`.dll`/`.dylib`) via `flutter_rust_bridge`, or a separate Rust daemon process on desktop |
| **Precedent** | This is close to how **AppFlowy** (open-source Notion/Obsidian-style app) is built — Flutter front end, Rust core for storage/sync. It's a proven combination at exactly this scale of app. |

**Why this is the strongest option:**
- Flutter genuinely ships one codebase to *all five* targets (mobile x2, desktop x3, web) with native performance and native-feeling widgets. React Native's desktop and web stories are bolted on (RN Windows/macOS, RN Web) and noticeably less mature/maintained than Flutter's.
- Rust is an excellent fit for the sync engine specifically: CRDT libraries are mature in Rust (`yrs` — the Rust port of Yjs, or `automerge-rs`), file-system watching is fast and safe, and the same compiled core can be reused unmodified across every platform, including as a small standalone server binary.
- File system access (reading/writing arbitrary folders, watching for changes) is something both Flutter and Rust handle well natively; this matters a lot for a "plain markdown files on disk" app.

**Downsides:**
- Dart is a smaller ecosystem than JS; fewer markdown-editor widgets exist off the shelf, so the rich text/markdown editor component will need more custom work (though this is true in RN too, honestly — see §7).
- Two languages in the codebase (Dart + Rust) with an FFI bridge is more moving parts than a pure-JS stack, though `flutter_rust_bridge` makes this fairly painless today.

### 4.3 Option B — React Native + React (web) + Tauri (desktop)

| | |
|---|---|
| **Mobile** | React Native (Expo) |
| **Web** | Plain React (Vite), sharing components with RN via React Native Web |
| **Desktop** | Tauri (Rust-backed shell, but the UI inside it is still your React/web code) |
| **Sync core** | TypeScript, or Rust exposed to all three via a common protocol (HTTP/WebSocket) rather than FFI |

**Why this is tempting:** you keep everything in one language (TypeScript) for the UI, which is a smaller hiring/mental-overhead surface than Flutter+Rust, and the markdown/rich-text editor ecosystem in JS is *huge* (ProseMirror, Lexical, CodeMirror 6, Milkdown, etc. — genuinely more mature than anything in Dart).

**Downsides:**
- This is actually **three separate frontend builds** stitched together (RN app, RN-Web-adapted React app, Tauri-wrapped web app), not one. They share components in principle, but you'll hit RN-Web compatibility gaps and end up special-casing things per platform anyway.
- File system access is inconsistent across the three: Tauri has full native FS access, RN needs platform-specific native modules (especially thorny on Android's scoped storage / Storage Access Framework), and plain web has essentially none (would need the File System Access API, which iOS Safari/WebView doesn't support well). You'd be solving the Android file-access problem three times over.
- More total surface area to maintain long-term for a project you're building and running solo.

### 4.4 Option C — Native per platform (Swift/SwiftUI + Kotlin/Compose + a web app)

Best possible platform integration and performance, but 3x (or 4x) the implementation and maintenance work for one person. Not recommended for v1 of a homelab side project — revisit only if this becomes something you want to polish extensively later.

### 4.5 Recommendation

**Go with Option A: Flutter for every client (Android, iOS, Windows/macOS/Linux, Web), with a Rust-based sync engine.**

Reasoning in one line: Flutter is the only option that gives you *one real codebase* across all five targets today with good native file-system access, and Rust is the right tool for a small, fast, memory-safe sync/CRDT engine that you can also run headless as the server binary in your homelab — meaning **client and server can share the exact same sync/merge logic**, compiled twice from one source. That shared-logic property is hard to overstate: it means conflict resolution behaves identically everywhere, and you write it once.

If you already know React well and don't know Dart at all, Option B is a defensible second choice — just go in aware that "one codebase" is more marketing than reality for RN across mobile+desktop+web, and budget extra time for the Android storage-permission model regardless of which UI framework you pick (that part of the pain is platform, not framework).

### 4.6 Server / deployment target

- Sync server: a single small Rust binary (Axum or similar for the HTTP/WebSocket layer) backed by a lightweight embedded store (SQLite for metadata + version history) for note metadata, with the `.md` files themselves as the canonical content store.
- **Vault storage decision (per your call above): the canonical vault lives on the server's own VM/LXC disk**, not on a device — this is now the one real copy of record, not just a relay. This makes durability of that storage more important than in a pure sync-relay design, so back this disk with your Proxmox storage that has redundancy (`local-lvm`, or better, back it onto the TrueNAS pool as the underlying storage for that VM's disk, giving you ZFS-level protection) and take regular snapshots.
- Runs as an **LXC container** on one of your Proxmox nodes (`pve-II`, alongside where Syncthing currently lives, is a natural fit) — LXC rather than a full VM since it's a single lightweight binary with no need for a full guest OS.
- Because devices only hold a *cache*, not a full mirror, the server is the only place that needs a real backup strategy — back up this one location (e.g. nightly snapshot or rsync off the LXC's disk to TrueNAS) and every device's data is covered.

---

## 5. Data Model

- **A note = one `.md` file.** File name is the note title (with the usual slugification for special characters), matching Obsidian's convention so vaults stay portable.
- **YAML frontmatter** at the top of each file for structured metadata:
  ```yaml
  ---
  id: 8f3a2c10-...        # stable UUID, assigned once, never reused — this is what sync tracks, not the filename
  created: 2026-08-05T10:00:00Z
  modified: 2026-08-05T10:04:12Z
  tags: [homelab, project]
  ---
  ```
- The `id` field is the key design decision: **sync tracks notes by a stable UUID in frontmatter, not by filename or path.** This means renaming or moving a note is just a metadata update, not a delete+create, and it's what makes rename/move safe across devices that sync at different times.
- Attachments live in a sibling `attachments/` (or per-note) folder and are referenced with standard markdown/relative links, synced as opaque binary blobs.
- Folder structure on disk mirrors the vault's folder tree exactly — no hidden indirection — so the vault is always readable/greppable/backupable as a normal directory of files, same as an Obsidian vault today.

---

## 6. Sync Design (Hybrid: Server-of-Record + Local Cache)

This is the part actually being built to replace Syncthing, so it deserves the most care. Given the hybrid model, this is simpler than a full peer-to-peer local-first design, because there is only ever **one** authoritative copy — the client cache never needs to be reconciled against *another device's* cache, only against the server.

- **Online behavior:** the client opens notes by fetching from the server (with the local cache as a fast first paint / offline fallback), and writes go straight to the server. A WebSocket connection pushes live updates so, e.g., editing the same note from two devices at once shows up promptly rather than silently diverging.
- **Offline behavior:** while offline, edits are held in a small local outbox (per-note diffs) and the note is served from cache for reading. On reconnect, the outbox is replayed against the server.
- **Conflict handling:** because reconciliation only ever happens between "my offline edits" and "the server's current version" (not N devices against each other), this can stay much simpler than full CRDT — a **per-note CRDT text merge (via `yrs`, the Rust Yjs port) applied once at reconnect** is enough: it merges your offline edits into whatever the server has, rather than needing to merge arbitrary devices pairwise. This is strictly better than Syncthing's "keep both files as `.sync-conflict` copies," while being noticeably simpler to implement than the full multi-device CRDT design a pure local-first app would need.
- **Cache policy (client-side):** cache recently/frequently opened notes plus, optionally, a user-pinned subset (e.g. "always keep my daily notes folder available offline") — not the whole vault by default, so mobile storage stays small. A "make available offline" toggle per note/folder is a good, cheap v1 feature given this design.
- Attachments (binary, non-mergeable) use simple last-write-wins by modification time; the server is authoritative here too, so there's no cross-device binary conflict to resolve, only "did my offline edit happen before or after the server's current version."
- Transport: WebSocket when online for near-real-time push/pull; falls back to HTTP polling/reconcile when the socket isn't available (mobile backgrounding, spotty wifi, VPN reconnects).

---

## 7. Editor

Recommend a **single hybrid markdown editor** for v1 (source-mode markdown with light live styling — bold/italic/headers rendered inline, not a separate WYSIWYG mode) rather than building both a strict source view and a rich WYSIWYG view. Obsidian's "Live Preview" mode is the right target to imitate; building two full editor modes is scope you don't need in v1.

- On the Flutter side, this will likely mean building on top of a code-editor-style widget (there isn't a mature drop-in "Obsidian editor" widget in Dart) — this is the single biggest custom-engineering item in the whole project and should be prototyped first, before investing in anything else, to de-risk the stack choice.

---

## 8. Platform Notes

- **Android**: cache lives in normal app-private storage (no SAF/user folder grant needed for v1, since the cache isn't meant to be a user-browsable vault copy) — this sidesteps the SMB/NFS problem entirely, since the phone never talks to the NAS directly, only to your sync server over HTTP/WebSocket.
- **iOS**: same approach — app sandbox storage for the cache, sync server as the only source of truth.
- **Desktop**: same client behavior as mobile (cache + server); optionally, desktop could *also* offer a "mount/watch a real folder" mode later as a power-user escape hatch, but that's not required for v1 given the hybrid design.
- **Web**: effectively a thin client already by nature of the hybrid model — reads/writes go through the server, with browser storage (IndexedDB) used for the same lightweight caching the native apps do.
- **Reachability requirement (new, given the hybrid model):** every client now needs a network path to the homelab to do anything beyond reading cached notes — worth deciding now whether that's Tailscale/WireGuard on each device, or a reverse-proxied endpoint exposed carefully (e.g. via your existing homelab ingress) with proper auth. This is the main new operational dependency this design introduces versus the original local-first plan.

---

## 9. Milestones

1. **Editor prototype** (Flutter, single platform e.g. desktop) — validate the markdown editing experience before anything else, since it's the highest-risk custom component.
2. **Local vault CRUD** — file tree, create/edit/delete notes and folders, all local, no sync yet, on desktop + one mobile platform.
3. **Sync server v0** — Rust binary on the homelab LXC, single client round-trip (write on desktop, confirm it's live on the server; read it back from a second client).
4. **Offline cache + reconnect merge** — kill connectivity on one client, edit, reconnect, verify the outbox replay/merge lands correctly against the server; then test the same with two clients editing concurrently while online.
5. **Search, tags, backlinks panel.**
6. **iOS + remaining desktop platforms + web client.**
7. **Polish**: settings, theming, attachment handling.

---

## 10. Open Questions

- Encryption at rest / in transit for the sync server: TLS in transit is assumed mandatory (even inside the homelab); encryption at rest on the server is a fast-follow decision, not v1-blocking, since the NAS already sits behind your network.
- Auth model for the sync server: simplest v1 answer is a single shared token/passphrase per device (this is a personal single-user tool, not multi-tenant) — full user accounts are unnecessary scope.
- Whether to also expose the vault read-only over the existing NAS share for tools like grep/ripgrep/backup scripts, in parallel with the sync protocol — likely yes, since files are just files.
