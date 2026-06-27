# caret — Design Guide

> The cursor is the product. caret turns a folder of Markdown files in a git repo into one
> live document you edit directly — no modes, no save dialogs, just fluid writing that saves
> back to clean, version-controlled files. This guide is the source of truth for the brand and
> the in-app design system. Keep it in the repo; it is meant to be read by humans and agents.

- **Status:** v1 · core brand + in-app system
- **Aesthetic axis:** minimal-technical, dark-first (Linear · Vercel · GitHub)
- **Hard rule:** the rendered app makes **zero external network requests** — fonts are
  system stacks or self-hosted `.woff2`, all icons are inline SVG. No CDNs, ever.

---

## 1. The one principle: no modality

Everything below serves this. **The page you look at is the document you edit.** There is no
view/edit/preview switch and no modal save flow. Design decisions are judged against it:

- The caret (insertion cursor) is always live. The product is named after it.
- Status is **ambient**, never a workflow. The UI signals *saved / saving / unsaved* without
  making the user "enter" or "exit" anything.
- Confidence is quiet. The only state allowed to be loud is **error**.
- Motion is functional — it reinforces "fluid/automatic," never decorates.

**Voice & tone:** precise, calm, builder-credible. Short sentences. Lowercase wordmark.
Say what a thing does, not how it feels.

| Do | Don't |
|---|---|
| "Saved." / "3 edits uncommitted." | "All your changes are safely stored! ✅" |
| Lowercase `caret` in product chrome | `Caret™ — Docs, Reimagined` |
| Let the dot carry the status | Toasts, spinners, modal "Saving…" overlays |
| One accent, used sparingly | Gradients, glows, multiple accent hues |

---

## 2. Brand

### Name & wordmark
- **Wordmark:** `caret`, lowercase, set in the **mono** stack, letter-spacing `-0.03em`.
- **App mark:** an upward chevron `^` (the caret glyph), 3.2–3.6 stroke weight, round joins,
  on a rounded-square tile (radius ≈ 25% of tile). Knockout color is a very dark tint of the
  accent (`#0B1B36`), never pure black.
- **Favicon:** same chevron. Filled-accent tile at 32/16px; an outline variant exists for
  monochrome contexts (chevron in accent on transparent).

```
App mark geometry (32×32 viewBox)
path: M7 20 L16 10 L25 20   stroke 3.2  linecap/linejoin round
tile: rounded-square, radius 14/56 ≈ 25%, fill = --accent
```

### Clear space & sizing
- Minimum clear space around the wordmark = the cap-height of the `c`.
- Minimum app-mark size: 16px. Below 16px, drop the tile and use the bare chevron.
- Never recolor the chevron outside the accent ramp or a neutral (`--fg` / `--muted`).

---

## 3. Color tokens

Implemented as CSS custom properties. **Dark is the default.** Light is an equal citizen,
toggled by `data-theme="light"` on `:root`/`body`. Tokens are semantic, not literal — never
hard-code a hex in a component; reference a token.

### Neutrals

```css
:root {                 /* dark — default */
  --canvas:   #0B0B0C;  /* app background, behind everything */
  --panel:    #141416;  /* cards, sidebar, header surfaces   */
  --panel-2:  #1A1A1D;  /* raised rows, hover fills          */
  --inset:    #0F0F11;  /* code blocks, wells, inputs        */

  --fg:       #ECECEE;  /* primary text, headings            */
  --fg-2:     #C7C7CC;  /* body / prose text                 */
  --muted:    #85858E;  /* secondary labels, metadata        */
  --faint:    #56565E;  /* tertiary, placeholder, disabled   */

  --border:   #262629;  /* default 1px separators            */
  --border-2: #33333A;  /* stronger / interactive borders    */
  --hair:     #1E1E21;  /* hairline dividers inside surfaces */
}

:root[data-theme="light"] {
  --canvas:   #EBEBEC;
  --panel:    #FFFFFF;
  --panel-2:  #F6F6F7;
  --inset:    #F1F1F2;

  --fg:       #16161A;
  --fg-2:     #3A3A40;
  --muted:    #6B6B73;
  --faint:    #9A9AA2;

  --border:   #E6E6E8;
  --border-2: #D8D8DC;
  --hair:     #EDEDEF;
}
```

### Accent — electric blue

One accent. Used for the caret, links, focus rings, active nav, and primary affordances only.

```css
:root {
  --accent:      #4C8DFF;  /* the caret blue                       */
  --accent-ink:  #0B1B36;  /* knockout text/icon ON accent fills   */
  --accent-soft: rgba(76,141,255,.12);  /* active-row / selection tint */
}
:root[data-theme="light"] {
  --accent:      #2F6FE6;  /* darkened for AA on white text/links  */
  --accent-ink:  #FFFFFF;
  --accent-soft: rgba(47,111,230,.10);
}
```

> **Link contrast:** on light surfaces use `--accent` (`#2F6FE6`, ≥ 4.5:1 on white). The
> brighter `#4C8DFF` is for dark surfaces and fills only.

### Semantic / ambient states

These are the **only** other colors in the system. They drive the status dots (§7) and nothing
decorative. Each is muted by design — confidence is quiet.

```css
:root {
  --state-saved:    #54B27D;  /* rendered HOLLOW (ring), at rest    */
  --state-unsaved:  #E0A458;  /* solid amber, edits pending         */
  --state-saving:   #6E9BF0;  /* pulsing dot, "a breath"            */
  --state-syncing:  #6E9BF0;  /* rippling ring, git commit (future) */
  --state-error:    #E0675E;  /* solid red — the only loud state    */
}
```

Light-mode values are identical; they already meet AA against `--panel`/`--canvas` in both
themes. If you ever place a state color on text, darken it ~12% in light mode.

---

## 4. Typography

Three stacks, zero required font files. All are self-hostable or system — never a web-font CDN.

```css
:root {
  --sans:  ui-sans-serif, system-ui, -apple-system, "Segoe UI", Helvetica, Arial, sans-serif;
  --mono:  ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace;
  --serif: "Iowan Old Style", "Palatino Linotype", Palatino, Georgia, serif;
}
```

- **`--sans`** — all UI chrome: sidebar, header, menus, labels, buttons.
- **`--mono`** — wordmark, code, keyboard hints (`⌘S`), token/metadata labels, code-fence
  language tags. Anything that is "machine."
- **`--serif`** — **prose body in the document surface only.** This is the editorial reading
  rhythm; it makes long-form writing feel like a page, not a form field. Headings in prose stay
  `--sans` for a crisp technical contrast.

> **Self-hosted upgrade path (optional):** if you want pixel-identical rendering across OSes,
> ship `Inter` (UI) + `JetBrains Mono` (code) + `Newsreader` or `Source Serif 4` (prose) as
> `.woff2` via `@font-face` and inline them. Keep the system stacks as the fallback list so the
> app still paints instantly before fonts load.

### Type scale

UI scale (sans) — tight, dense, Linear-like:

| Token | Size / line | Weight | Use |
|---|---|---|---|
| `ui-xs`   | 11 / 1   | 500 | mono labels, kbd hints, metadata (often `letter-spacing:.1em` uppercase) |
| `ui-sm`   | 12 / 1.4 | 500 | sidebar items, status text, menu rows |
| `ui-base` | 13 / 1.5 | 400 | breadcrumbs, secondary body |
| `ui-md`   | 14 / 1.5 | 500–600 | buttons, section headers |

Prose scale (document surface):

| Element | Font | Size / line | Weight | Tracking |
|---|---|---|---|---|
| H1 | sans  | 30 / 1.15 | 600 | -0.02em |
| H2 | sans  | 22 / 1.20 | 600 | -0.018em |
| H3 | sans  | 17 / 1.3  | 600 | -0.01em |
| Body | serif | 16 / 1.7 | 400 | 0 |
| Inline code | mono | 0.88em / inherit | 400 | 0 |
| Code block | mono | 13 / 1.7 | 400 | 0 |
| Blockquote | serif | 16 / 1.7 | 400 (italic optional) | 0 |

- Prose measure: cap line length at **62–68ch**. Never full-bleed text.
- `text-wrap: pretty` on headings and paragraphs.

---

## 5. Spacing, radius, elevation, border

### Spacing — 4px base

```
2  4  6  8  10  12  14  16  20  24  28  32  40  48  64
```

Use `gap` on flex/grid for any group of siblings (nav rows, toolbar buttons, dot+label). Never
space UI elements with bare inline flow or per-element margins. Document surface uses the larger
end (28–48); chrome uses the smaller end (6–16). **The writing surface stays spacious even
though the chrome is dense.**

### Radius

```css
--r-xs: 5px;   /* kbd hints, tiny chips      */
--r-sm: 6px;   /* menu rows, sidebar items   */
--r-md: 8px;   /* buttons, inputs, swatches  */
--r-lg: 10px;  /* code blocks, callouts      */
--r-xl: 14px;  /* cards, panels, app-mark tile */
```

Not everything is rounded. Hairlines, dividers, and the writing column have square corners.
Rounded-everything reads as consumer SaaS — avoid.

### Border & elevation

- Default separators: `1px solid var(--border)`. Inside a surface, use `var(--hair)`.
- Interactive/raised edges: `var(--border-2)`.
- **Elevation is restrained.** Dark mode leans on borders + near-black fills, not big shadows.

```css
/* dark */
--shadow-sm: 0 1px 2px rgba(0,0,0,.25);
--shadow-md: 0 1px 2px rgba(0,0,0,.25), 0 18px 50px -24px rgba(0,0,0,.5);
/* light */
--shadow-sm-l: 0 1px 2px rgba(16,16,26,.06);
--shadow-md-l: 0 1px 2px rgba(16,16,26,.06), 0 12px 32px -16px rgba(16,16,26,.18);
```

Popovers (slash menu, formatting toolbar) use `--shadow-md`; flat chrome uses `--shadow-sm` or
nothing.

---

## 6. Motion

Subtle and functional. Respect `prefers-reduced-motion: reduce` — disable all of it.

```css
--ease:      cubic-bezier(.2, .6, .2, 1);   /* default UI            */
--ease-out:  cubic-bezier(.16, 1, .3, 1);   /* enters, expands       */
--dur-1: 120ms;   /* hovers, focus rings, micro-state */
--dur-2: 160ms;   /* menus, toggles, row selection    */
--dur-3: 220ms;   /* panel/sidebar collapse           */
```

Three signature keyframes — used *only* by the status system and the caret:

```css
@keyframes breathe { 0%,100%{opacity:.35;transform:scale(.85)} 50%{opacity:1;transform:scale(1)} }
@keyframes ring    { 0%{transform:scale(.6);opacity:.7} 100%{transform:scale(2.2);opacity:0} }
@keyframes blink   { 0%,49%{opacity:1} 50%,100%{opacity:0} }
```

- **`breathe`** (1.6s) → the *saving* dot. A breath, never a spinner.
- **`ring`** (1.4s) → the *syncing* / git-commit ripple.
- **`blink`** (1.1s, `step-end`) → the caret itself.

```css
@media (prefers-reduced-motion: reduce) { * { animation: none !important; transition: none !important; } }
```

---

## 7. The ambient status system

The defining surface of the product. **One dot language** carries both the document's save state
and (roadmap) the repo's commit/sync state, side by side, with no second competing system.

| State | Dot treatment | Color | Meaning |
|---|---|---|---|
| **Saved**   | hollow ring (1.5px) | `--state-saved`   | file matches disk; at rest |
| **Unsaved** | solid               | `--state-unsaved` | edits pending · `⌘S` or auto |
| **Saving**  | solid, `breathe`    | `--state-saving`  | persisting — a breath |
| **Syncing** | ring, `ring` ripple | `--state-syncing` | committing to git *(future)* |
| **Error**   | solid               | `--state-error`   | save failed — the only loud state |

**Manual vs auto-save — same calm system, not two apps:**

- **Manual (default):** a quiet `Save` affordance + `⌘/Ctrl+S`. Dot sits at *unsaved* (amber)
  while dirty, briefly *saving* (breath), then settles to *saved* (hollow). An ambient "unsaved
  changes" marker also appears in the sidebar on the dirty page.
- **Auto-save (opt-in toggle):** edits persist debounced. The `Save` affordance recedes; the dot
  does the same *unsaved → saving → saved* arc on its own. **No spinner, no toast.** The toggle
  lives in Settings (§8) and as an inline control in the header.

**Doc + repo in one row** (anticipating git management): the save dot and the sync dot read as a
single status, e.g. `● main.md · saved   ◌ 3 edits uncommitted`. They share the dot vocabulary,
sizes (7–9px), and spacing — never introduce a second pattern (badges, pills, banners) for git.

**History/timeline (concept):** a lightweight affordance to scrub past versions of a doc lives
*next to* the sync dot, surfaced ambiently (a small timestamp/`⌥` reveal), not as a separate
mode or full screen.

---

## 8. Components & screens

Every interactive surface must define all states: **default / hover / focus / active / disabled
/ loading / empty / error.** Focus is always a visible 2px `--accent` ring (`box-shadow:
0 0 0 2px var(--canvas), 0 0 0 4px var(--accent)` so it reads on any surface).

### Document surface (the soul)
- The writing column is centered, 62–68ch, generous vertical rhythm. Square corners on the
  column; no card around the text.
- Full prose styling per §4: H1–H3 (sans), serif body, mono inline/block code, callouts,
  blockquotes (accent or state-colored 3px left rule), tables, checklists, dividers (hairline),
  images (rounded `--r-lg`, 1px border).
- Code blocks: `--inset` fill, `--r-lg`, a header strip with a square language chip + filename
  in mono. Syntax highlighting uses the accent + state hues only (keyword = iris/violet,
  function = accent, comment = `--faint`), never a rainbow theme.
- Internal links: `--accent` text + a low-opacity accent underline, **pointer cursor**, navigate
  in-app on click (no popup).
- **Empty page:** a single faint caret prompt + placeholder ("Start writing, or press `/`").
  No illustration.

### Inline formatting toolbar (re-theme existing)
- Floating popover on selection: `--panel`, `--border-2`, `--r-md`, `--shadow-md`.
- Icon buttons, 28–32px hit area min, `--muted` default → `--fg` on hover with `--panel-2` fill →
  `--accent` + `--accent-soft` when active (bold/italic applied). 120ms.

### Slash (`/`) command menu (re-theme existing)
- Popover same shell as toolbar. Rows: icon (block type) + label + mono shortcut on the right.
- Active row: `--accent-soft` fill, `--fg` text. Group headings in `ui-xs` mono uppercase
  `--faint`. Filtering is instant; empty filter → "No blocks" in `--muted`.

### Sidebar / navigation tree
- `--panel` surface, `1px --border` against `--canvas`. Dense rows (`ui-sm`, ~28px tall).
- Folder/file rows: inline-SVG folder/doc icon in `--muted`; active doc = `--accent-soft` fill +
  `--fg` text; hover = `--panel-2`.
- **Dirty marker:** a 6px `--state-unsaved` dot left of the filename on any page with unsaved
  changes — same dot language as the header.
- Collapse: chevrons rotate with `--dur-3`. Search field at top (`--inset`, mono placeholder).
- **Empty workspace:** "No pages yet" + a quiet "New page" action; no marketing.

### Save affordance & auto-save toggle
- `Save` is a quiet text/icon button in the header, `--muted` → `--fg` on hover; shows `⌘S` kbd
  hint in mono. Recedes (opacity, not removal) when auto-save is on.
- Toggle: 34×20 track, `--inset` off / `--accent` on, 14px knob, `--dur-2`. Labeled "Auto-save".

### Settings / preferences
- Calm two-column form on `--panel`: section labels in `ui-xs` mono uppercase `--faint`, rows
  with title + one-line description + control on the right. Houses the auto-save toggle, theme,
  font preferences, and future git options. No tabs unless it outgrows one screen.

### Error states
- **Save failed:** the only loud moment — `--state-error` dot + inline "Couldn't save · Retry"
  in the header, not a modal. Keep the editor fully usable.
- **Editor failed to load:** full-surface fallback with the bare chevron, a one-line message, and
  a Reload action.
- **Offline:** sync dot goes `--faint`/hollow with "offline" label; editing continues locally.

### Published / read-only view
- The authoring app *is* the reading view — just drop the editor chrome (toolbar, slash menu,
  save/sync affordances, dirty markers). Same tokens, same prose styling, same sidebar for
  navigation. It should feel like the same calm page, frozen.

---

## 9. Iconography

Inline SVG only — no icon-font, no hosted kit. One consistent grammar:

- **Grid:** 16px viewBox for chrome icons, 32px for the app mark.
- **Stroke:** 1.2–1.5px for 16px icons, round caps/joins, `currentColor` so they inherit text
  color and invert with theme automatically.
- **Coverage:** page/doc, folder, chevron (caret/collapse), search, settings (gear), the slash-menu
  block types (heading, list, checklist, table, code, quote, image, divider), and the save/sync/
  git state dots (which are CSS, not icons).
- Accent fills only on the app mark and active states; everywhere else icons are neutral.

---

## 10. Accessibility baseline

- **Contrast AA+:** body text ≥ 4.5:1, large/UI text ≥ 3:1, in **both** themes. (Link accent is
  pre-tuned per theme — see §3.)
- **Focus:** always visible, 2px accent ring with a `--canvas` halo so it reads on any surface.
  Never remove outlines without replacing them.
- **Keyboard:** everything reachable and operable; `⌘/Ctrl+S` saves; slash menu and toolbar are
  arrow-navigable; Esc dismisses popovers.
- **Reduced motion:** `prefers-reduced-motion` disables breathe/ring/blink and all transitions.
- **Status is never color-only:** every dot pairs with a text label or `aria-live` announcement
  ("Saved", "Saving", "Save failed") so the ambient system is legible to screen readers too.

---

## 11. Anti-goals (will get rejected)

- Reintroducing **modes** — edit/preview/read switches, modal save flows.
- **Generic SaaS** look — gratuitous gradients, stock illustration, rounded-everything.
- **External dependencies** that break self-containment — web-font CDNs, hosted icon kits.
- **Loud** save/loading — spinners-in-your-face, "Saving…" overlays, success toasts.
- Over-decorated chrome that competes with the writing surface. When in doubt, remove it.

---

*caret · design guide · v1 — keep this file in the repo. Agents and humans both read it.*
