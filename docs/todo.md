# caret (docd) — TODO

## Shipped

- [x] **Auto-save** — opt-in (debounced), routed through the normalizer; manual
  `⌘S` still works. Toggle in header + Settings.
- [x] **Follow inline links** — plain click navigates (internal `.md` → route,
  external → new tab); no popup.
- [x] **Per-repo settings + secrets** — `.docd/config.toml` (committed) and
  `.docd/secrets.toml` (gitignored, write-only), editable in the Settings drawer.
- [x] **caret design system** — tokens (dark/light), ambient status dots, page sheet.
- [x] **Page-sheet + book** — seamless paper-width document; **Ctrl/Cmd+P prints
  the whole repo as one PDF** (no separate command — CLI stays init/build/serve).

## Print / book follow-ups

- [ ] **A4 / configurable paper + margins** — currently Letter via `--page-w`;
  make it a per-repo setting (the CSS vars already exist).
- [ ] **Book niceties** — optional title page + table of contents; page numbers
  in a running footer (`@page` margin boxes).
- [ ] **True vertical pagination** — page-for-page on screen (paged.js). Only if
  the width-match proves insufficient; deferred per product call.

## Editing UX

- [ ] **Intra-page heading anchors** — routing is page-level; `page.md#section`
  fragments are currently dropped.

## Platform

- [ ] **Auto-commit on save** — `git add` + commit each save so editing produces real
  history end-to-end.
- [ ] **Create / rename / delete pages** from the GUI (`serve` is edit-only today).
- [ ] **gzip the serve response** — the inlined editor makes pages ~2.8MB; gzip cuts
  wire size ~4×. Do this if page load feels heavy.
- [x] **Persist referenced images** — hover an external image → "Save to repo" →
  downloaded into `img/`, markdown rewritten to the local path, served at `/img/*`.
- [x] **Excalidraw diagrams** — diagrams are images (`![excalidraw:<id>](img/diagram-<id>.svg)`),
  editable source in `diagrams/<id>.excalidraw`; editing never touches the `.md`. Excalidraw
  bundle embedded + served at `/_excalidraw.js` (lazy, off the page). *Needs an in-browser eyeball.*
- [ ] **Excalidraw follow-ups** — vendor Excalifont (restore hand-drawn look; currently system
  fallback); at release, swap `include_str!` → fetch-from-release + cache (lean binary, "clean C").
- [ ] **AI refactor** — run prompts against the raw markdown dir from the UI.
- [ ] **External-change auto-reload** — watch the repo's `.md` files for edits made
  outside the UI (e.g. an AI agent or `git pull` rewriting files) and live-reload the
  affected page in the browser. Notify/merge gracefully if the open doc has unsaved
  edits (don't clobber). Likely a file watcher + SSE/websocket push from `serve`.
