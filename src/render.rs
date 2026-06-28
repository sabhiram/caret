use anyhow::Result;
use regex::Regex;
use serde::Serialize;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Serialize)]
pub struct Page {
    pub slug: String,
    pub title: String,
    pub order: i64,
    pub html: String,
    /// Raw frontmatter block (incl. delimiters + trailing newline), or "" if none.
    /// Re-attached on save so the editor never has to show YAML.
    pub frontmatter: String,
    /// Markdown body without frontmatter — what the WYSIWYG editor loads.
    pub body: String,
}

/// Split a raw file into (frontmatter block, body). The frontmatter keeps its
/// `---` delimiters and trailing newline so `frontmatter + body == raw`.
pub fn split_frontmatter(raw: &str) -> (String, String) {
    if let Some(after_open) = raw.strip_prefix("---\n") {
        if let Some(idx) = after_open.find("\n---") {
            let rest = &after_open[idx + 4..];
            if let Some(nl) = rest.find('\n') {
                let split_at = 4 + idx + 4 + nl + 1;
                return (raw[..split_at].to_string(), raw[split_at..].to_string());
            }
            return (raw.to_string(), String::new());
        }
    }
    (String::new(), raw.to_string())
}

/// "guide/getting-started.md" -> "guide/getting-started"
pub fn slug_for(rel: &str) -> String {
    let r = rel.replace('\\', "/");
    r.strip_suffix(".md").unwrap_or(&r).to_string()
}

/// Resolve a relative `.md` link, as seen from `from_slug`, into an in-page
/// route like `#/guide/concepts`. External / anchor / non-md links pass through.
pub fn resolve_link(from_slug: &str, href: &str) -> String {
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || href.starts_with('#')
        || href.starts_with('/')
    {
        return href.to_string();
    }
    let path_part = href.split('#').next().unwrap_or(href);
    if !path_part.to_ascii_lowercase().ends_with(".md") {
        return href.to_string();
    }
    // Routing is page-level, so intra-page heading fragments are dropped for now.
    let target = path_part.strip_suffix(".md").unwrap_or(path_part);

    let mut stack: Vec<&str> = Vec::new();
    if let Some((dir, _)) = from_slug.rsplit_once('/') {
        stack.extend(dir.split('/'));
    }
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            s => stack.push(s),
        }
    }
    format!("#/{}", stack.join("/"))
}

/// Rewrite every `href="..."` in rendered HTML through `resolve_link`.
fn rewrite_links(html: &str, from_slug: &str) -> String {
    // Operates on rendered HTML; can move to a comrak AST walk once editing needs the AST.
    let re = Regex::new(r#"href="([^"]*)""#).unwrap();
    re.replace_all(html, |c: &regex::Captures| {
        format!("href=\"{}\"", resolve_link(from_slug, &c[1]))
    })
    .into_owned()
}

/// Read `title`/`order` from a frontmatter block (the `---`-delimited header that
/// `split_frontmatter` returns). Minimal `key: value` parser; "" yields defaults.
fn frontmatter_meta(fm: &str) -> (Option<String>, i64) {
    let mut title = None;
    let mut order = 999;
    for line in fm.lines() {
        if line.trim() == "---" {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            match k.trim() {
                "title" => title = Some(v.to_string()),
                "order" => order = v.parse().unwrap_or(999),
                _ => {}
            }
        }
    }
    (title, order)
}

fn first_h1(body: &str) -> Option<String> {
    body.lines()
        .find_map(|l| l.strip_prefix("# ").map(|s| s.trim().to_string()))
}

fn ignored(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some(".git") | Some("node_modules") | Some("target") | Some("dist")
        )
    })
}

/// The one set of markdown options used everywhere — render AND normalize.
/// front_matter_delimiter keeps `---` frontmatter out of rendered HTML and
/// preserved verbatim on normalize.
fn base_opts() -> comrak::Options<'static> {
    let mut o = comrak::Options::default();
    o.extension.table = true;
    o.extension.strikethrough = true;
    o.extension.autolink = true;
    o.extension.tasklist = true;
    o.extension.front_matter_delimiter = Some("---".to_string());
    o
}

/// The single deterministic write-path: parse -> re-emit canonical CommonMark.
/// Every save (textarea now, WYSIWYG/AI later) funnels through here, so files
/// converge to one format and git diffs stay minimal. Idempotent by design.
pub fn normalize_markdown(raw: &str) -> String {
    let arena = comrak::Arena::new();
    let opts = base_opts();
    let root = comrak::parse_document(&arena, raw, &opts);
    let mut out = Vec::new();
    comrak::format_commonmark(root, &opts, &mut out).expect("format_commonmark");
    let out = String::from_utf8(out).expect("utf8");
    strip_list_separators(&out)
}

/// comrak emits `<!-- end list -->` between two adjacent lists so they don't merge
/// on re-parse. It's noise that renders as literal text in the editor, so drop it
/// (and collapse the blank lines it leaves behind).
fn strip_list_separators(md: &str) -> String {
    if !md.contains("<!-- end list -->") {
        return md.to_string();
    }
    let kept: Vec<&str> = md
        .lines()
        .filter(|l| l.trim() != "<!-- end list -->")
        .collect();
    let mut result = kept.join("\n");
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Render a full markdown file (with frontmatter) to the page HTML used by the
/// reading view and the print book — the same pipeline as `collect_pages`.
pub fn render_html(raw: &str, slug: &str) -> String {
    rewrite_links(&comrak::markdown_to_html(raw, &base_opts()), slug)
}

/// Remove diagram files (sidecar `.excalidraw` + rendered `.svg`) that no markdown
/// doc references. The app is the only thing that creates diagrams, so anything
/// unreferenced is a leftover from an insert that wasn't kept. Returns # removed.
pub fn gc_orphan_diagrams(root: &Path) -> usize {
    let re = Regex::new(r"diagram-([A-Za-z0-9_-]+)\.svg").unwrap();
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if entry.file_type().is_file()
            && !ignored(p)
            && p.extension().and_then(|s| s.to_str()) == Some("md")
        {
            if let Ok(c) = std::fs::read_to_string(p) {
                for cap in re.captures_iter(&c) {
                    referenced.insert(cap[1].to_string());
                }
            }
        }
    }
    let mut removed = 0;
    if let Ok(rd) = std::fs::read_dir(root.join("diagrams")) {
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) == Some("excalidraw") {
                if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                    if !referenced.contains(id) && std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(root.join("img")) {
        for e in rd.flatten() {
            let path = e.path();
            if let Some(id) = path
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(|n| n.strip_prefix("diagram-"))
                .and_then(|n| n.strip_suffix(".svg"))
            {
                if !referenced.contains(id) && std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    removed
}

pub fn collect_pages(root: &Path) -> Result<Vec<Page>> {
    let opts = base_opts();

    let mut pages = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !entry.file_type().is_file() || ignored(path) {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let rel = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let slug = slug_for(&rel);
        let raw = std::fs::read_to_string(path)?;
        let (frontmatter, body) = split_frontmatter(&raw);
        let (fm_title, order) = frontmatter_meta(&frontmatter);
        // Strip the `<!-- end list -->` artifact so the editor never displays it,
        // even if an older save left one in the file.
        let body = strip_list_separators(&body);
        // Render the whole file: front_matter_delimiter drops the frontmatter from HTML.
        let html = rewrite_links(&comrak::markdown_to_html(&raw, &opts), &slug);
        let title = fm_title
            .or_else(|| first_h1(&body))
            .unwrap_or_else(|| slug.clone());
        pages.push(Page {
            slug,
            title,
            order,
            html,
            frontmatter,
            body,
        });
    }
    pages.sort_by(|a, b| a.order.cmp(&b.order).then(a.slug.cmp(&b.slug)));
    Ok(pages)
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn esc_attr(s: &str) -> String {
    esc_html(s).replace('"', "&quot;")
}

pub fn build_html(pages: &[Page], editable: bool, theme: &str, settings_json: &str) -> String {
    // Escape `<` so embedded page HTML can't break out of the <script> tag.
    let data = serde_json::to_string(pages)
        .unwrap()
        .replace('<', "\\u003c");
    let nav = pages
        .iter()
        .map(|p| {
            let depth = p.slug.matches('/').count();
            format!(
                r##"<a href="#/{slug}" data-slug="{slug}" class="navrow" style="padding-left:{pad}px"><svg class="ico" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6"/></svg><span class="lbl">{title}</span></a>"##,
                slug = esc_attr(&p.slug),
                pad = 10 + depth * 14,
                title = esc_html(&p.title),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let theme = if theme == "light" { "light" } else { "dark" };
    TEMPLATE
        .replace("__THEME__", theme)
        .replace("__NAV__", &nav)
        .replace("__DATA__", &data)
        .replace("__SETTINGS__", settings_json)
        .replace("__HELP_HTML__", &render_html(HELP_BODY, "help"))
        .replace("__EDITABLE__", if editable { "true" } else { "false" })
        .replace(
            "__EDITOR_ASSETS__",
            if editable { editor_assets() } else { "" },
        )
        .replace("__EDITOR_JS__", if editable { EDITOR_JS } else { "" })
}

/// The built-in welcome/help guide — used as the `init` welcome page AND shown by
/// the `?` button in any project (single source of truth).
pub const HELP_BODY: &str = r##"# Using caret

caret turns a folder of Markdown files you own into a live, editable site — edits
save straight back to disk (and git).

## Editing

- The page *is* the editor — just start typing. There are no edit/preview modes.
- Press **/** for a block menu (headings, lists, tables, code, quotes).
- Save with **⌘S**, or turn on **Auto-save** in Settings (the gear).
- Links between pages navigate in-app — click one to follow it.

## Managing pages

- **+ New page** at the bottom of the sidebar — type a path like `guide/setup` to
  create folders as you go.
- Hover a page in the sidebar to **rename / move** it or **delete** it.

## Diagrams

- Click the **◇** button in the top bar to insert an Excalidraw diagram.
- Hover a diagram to **edit** it. The drawing lives in `diagrams/` and renders to
  `img/`, so editing a diagram never bloats your Markdown.

## Images

- Reference any image. Hover an external one to **save it to the repo** so it lives
  in `img/` and works offline.

## Printing

- **⌘P** prints the *whole repo* as one clean PDF.

## Settings (the gear)

- Project **title**, **theme** (light/dark), **auto-save**, and **API keys** —
  keys are stored in `.caret/secrets.toml`, which is gitignored and never committed.

---

*It's all plain Markdown in your git repo. Own it, commit it, hand it to an AI
agent — it's just files.*
"##;

// Vendored Milkdown (Crepe) editor, inlined so a served page makes ZERO network
// requests. Built from assets/ via esbuild; see assets/README for the recipe.
const MILKDOWN_CSS: &str = include_str!("../assets/milkdown.bundle.css");
const MILKDOWN_JS: &str = include_str!("../assets/milkdown.bundle.js");

/// CSS + JS for the editor, inlined into <head>. Only emitted in `serve` mode.
/// Computed once: the JS bundle is ~2.7MB, so we avoid re-scanning it per request.
fn editor_assets() -> &'static str {
    static ASSETS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ASSETS.get_or_init(|| {
        // Neutralize any literal </style> or </script> so inlined assets can't
        // close their host tag early.
        let css = MILKDOWN_CSS.replace("</style", "<\\/style");
        let js = MILKDOWN_JS.replace("</script", "<\\/script");
        format!("<style>{css}</style>\n<script>{js}</script>")
    })
}

// Editor wiring, injected only in `serve` mode. The Milkdown bundle is already
// inlined in <head> and exposes window.DocdEditor — this just mounts it, tracks
// the dirty state, and guards against losing unsaved edits.
const EDITOR_JS: &str = r##"
var handle = null, editingPage = null, autoTimer = null;
function markRow(d){
  if (!editingSlug) return;
  var link = document.querySelector('#sidebar .navrow[data-slug="' + (window.CSS ? CSS.escape(editingSlug) : editingSlug) + '"]');
  if (link) link.classList.toggle("dirty", d);
}
function setDirty(d){
  dirty = d;
  setStatus(d ? "unsaved" : "saved");
  window.onbeforeunload = d ? function(e){ e.preventDefault(); e.returnValue = ""; return ""; } : null;
  markRow(d);
}
function scheduleAutoSave(){
  if (!AUTO_SAVE) return;
  if (autoTimer) clearTimeout(autoTimer);
  autoTimer = setTimeout(function(){ saveCurrent(); }, 1200);
}
function onEdit(isDirty){
  setDirty(isDirty);
  if (isDirty) scheduleAutoSave();
}
function saveCurrent(){
  if (!handle || !dirty) return;
  if (autoTimer){ clearTimeout(autoTimer); autoTimer = null; }
  setStatus("saving");
  var markdown = editingPage.frontmatter + handle.getMarkdown() + "\n";
  fetch("/api/save", {method:"POST", headers:{"Content-Type":"application/json"},
    body: JSON.stringify({slug: editingPage.slug, markdown: markdown})})
    .then(function(r){ return r.json(); })
    .then(function(j){
      if (j.ok){
        editingPage.body = handle.getMarkdown(); handle.markSaved();
        dirty = false; window.onbeforeunload = null; markRow(false); setStatus("saved");
        // refresh the print book with the freshly-rendered HTML (incl. new diagrams)
        if (typeof j.html === "string"){ editingPage.html = j.html; if (typeof buildBook === "function") buildBook(); }
      }
      else { setStatus("error"); }
    })
    .catch(function(){ setStatus("error"); });
}
// Resolve a relative .md link (from the editing page) to an in-app route.
// Mirrors the Rust resolve_link so the editor and `build` navigate identically.
function resolveMdHref(fromSlug, href){
  var pathPart = href.split("#")[0].replace(/\.md$/i, "");
  var stack = fromSlug.indexOf("/") >= 0 ? fromSlug.split("/").slice(0, -1) : [];
  pathPart.split("/").forEach(function(seg){
    if (seg === "" || seg === ".") { /* skip */ }
    else if (seg === "..") { stack.pop(); }
    else { stack.push(seg); }
  });
  return "#/" + stack.join("/");
}
// In the editable surface a plain click on a link navigates (no popup):
// internal .md -> in-app route, external http -> new tab.
function onEditorLinkClick(e){
  var a = e.target.closest ? e.target.closest("a") : null;
  if (!a) return;
  var href = a.getAttribute("href");
  if (!href) return;
  if (/^https?:/i.test(href)){ e.preventDefault(); e.stopPropagation(); window.open(href, "_blank", "noopener"); return; }
  var pathPart = href.split("#")[0];
  if (!/\.md$/i.test(pathPart)) return; // mailto:, in-page anchors, etc. -> default
  e.preventDefault(); e.stopPropagation();
  location.hash = resolveMdHref(editingSlug, href);
}
// The page IS the editor — mount Crepe directly into the content area.
function editPage(page){
  editingPage = page; editingSlug = page.slug;
  if (handle){ try { handle.destroy(); } catch(e){} handle = null; }
  if (autoTimer){ clearTimeout(autoTimer); autoTimer = null; }
  content.innerHTML = "";
  var host = document.createElement("div"); host.id = "editor";
  content.append(host);
  host.addEventListener("click", onEditorLinkClick, true);
  setDirty(false);
  window.DocdEditor.mount(host, page.body, onEdit)
    .then(function(h){ handle = h; })
    .catch(function(){ host.innerHTML = '<div class="loadfail">Editor failed to load. <button class="btn" onclick="location.reload()">Reload</button></div>'; });
}
addEventListener("keydown", function(e){
  if ((e.metaKey || e.ctrlKey) && (e.key === "s" || e.key === "S")){ e.preventDefault(); saveCurrent(); }
  else if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K") && handle){ e.preventDefault(); openLinkPicker(); }
});
// --- ⌘K: link selected text to another page ---
// Relative `.md` path from one slug to another (mirrors the in-app resolver).
function relMdPath(fromSlug, toSlug){
  var f = fromSlug.split("/").slice(0, -1);
  var t = toSlug.split("/");
  var i = 0;
  while (i < f.length && i < t.length - 1 && f[i] === t[i]) i++;
  var rel = [];
  for (var k = i; k < f.length; k++) rel.push("..");
  for (var k = i; k < t.length; k++) rel.push(t[k]);
  return rel.join("/") + ".md";
}
function openLinkPicker(){
  if (!handle || !handle.setLink) return;
  var scrim = document.createElement("div"); scrim.className = "lp-scrim";
  var box = document.createElement("div"); box.className = "lp-box";
  var inp = document.createElement("input"); inp.className = "lp-input"; inp.placeholder = "Link to a page, or paste a URL…"; inp.spellcheck = false;
  var list = document.createElement("div"); list.className = "lp-list";
  box.appendChild(inp); box.appendChild(list); scrim.appendChild(box); document.body.appendChild(scrim);
  var sel = 0, items = [];
  function close(){ scrim.remove(); }
  function isUrlish(q){ return /^(https?:\/\/|\/|\.\.?\/)/i.test(q) || /\.[a-z]{2,4}($|[?#])/i.test(q); }
  function build(){
    var q = inp.value.trim();
    items = [];
    if (q && isUrlish(q)) items.push({ url: q, label: "Link to: " + q });
    var ql = q.toLowerCase();
    PAGES.forEach(function(p){
      if (p.slug === editingSlug) return;
      if (!q || (p.title + " " + p.slug).toLowerCase().indexOf(ql) >= 0) items.push({ page: p });
    });
    if (sel >= items.length) sel = items.length - 1; if (sel < 0) sel = 0;
    list.innerHTML = "";
    items.forEach(function(it, i){
      var row = document.createElement("div"); row.className = "lp-row";
      if (it.url){ var s=document.createElement("span"); s.className="lp-title"; s.textContent=it.label; row.appendChild(s); }
      else { var a=document.createElement("span"); a.className="lp-title"; a.textContent=it.page.title; var b=document.createElement("span"); b.className="lp-slug"; b.textContent=it.page.slug; row.appendChild(a); row.appendChild(b); }
      row.addEventListener("mousedown", function(e){ e.preventDefault(); pick(it); });
      row.addEventListener("mouseenter", function(){ sel = i; paint(); });
      list.appendChild(row);
    });
    paint();
  }
  function paint(){ var rows = list.children; for (var i=0;i<rows.length;i++) rows[i].classList.toggle("sel", i===sel); }
  function pick(it){
    if (it.url){ handle.setLink(it.url, it.url); }
    else { handle.setLink(relMdPath(editingSlug, it.page.slug), it.page.title); }
    onEdit(true); close();
  }
  inp.addEventListener("input", function(){ sel = 0; build(); });
  inp.addEventListener("keydown", function(e){
    if (e.key === "ArrowDown"){ e.preventDefault(); sel = Math.min(sel+1, items.length-1); paint(); }
    else if (e.key === "ArrowUp"){ e.preventDefault(); sel = Math.max(sel-1, 0); paint(); }
    else if (e.key === "Enter"){ e.preventDefault(); if (items[sel]) pick(items[sel]); }
    else if (e.key === "Escape"){ e.preventDefault(); close(); }
  });
  scrim.addEventListener("mousedown", function(e){ if (e.target === scrim) close(); });
  build(); inp.focus();
}
// Hover an external image -> a subtle "Save to repo" button that localizes it.
var imgPop = null, imgHideT = 0;
function isExternalImg(src){ return /^https?:\/\//i.test(src) && src.indexOf(location.origin + "/") !== 0; }
function hideImgPop(){ imgHideT = setTimeout(function(){ if (imgPop) imgPop.style.display = "none"; }, 220); }
function diagramId(img){
  var a = img.getAttribute("alt") || "";
  if (/^excalidraw:/.test(a)) return a.slice(11);
  // The editor may not keep the alt on the <img>, so identify by the src too.
  var m = (img.getAttribute("src") || "").match(/diagram-([A-Za-z0-9_-]+)\.svg/);
  return m ? m[1] : null;
}
var ICON_EDIT = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z"/></svg>';
var ICON_SAVE = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/></svg>';
function ensureImgPop(){
  if (imgPop) return imgPop;
  imgPop = document.createElement("div"); imgPop.id = "imgactions";
  document.body.appendChild(imgPop);
  imgPop.addEventListener("mouseenter", function(){ clearTimeout(imgHideT); });
  imgPop.addEventListener("mouseleave", hideImgPop);
  return imgPop;
}
function iconBtn(svg, title, onClick){
  var b = document.createElement("button"); b.className = "iconbtn"; b.type = "button";
  b.title = title; b.setAttribute("aria-label", title); b.innerHTML = svg;
  b.addEventListener("click", onClick);
  return b;
}
function showImgActions(img){
  var p = ensureImgPop(); clearTimeout(imgHideT); p.innerHTML = "";
  var id = diagramId(img);
  if (id){ p.appendChild(iconBtn(ICON_EDIT, "Edit diagram", function(){ hideImgPop(); editDiagram(id); })); }
  else if (isExternalImg(img.getAttribute("src") || "")){ p.appendChild(iconBtn(ICON_SAVE, "Save to repo", function(){ saveImageToRepo(img); })); }
  else { p.style.display = "none"; return; }
  p.style.display = "flex";
  var r = img.getBoundingClientRect();
  var left = r.right + 8;
  if (left + p.offsetWidth > window.innerWidth - 8) left = Math.max(8, r.right - p.offsetWidth - 8);
  p.style.top = (r.top + 6) + "px"; p.style.left = left + "px";
}
function saveImageToRepo(img){
  var url = img.getAttribute("src"); if (!url || !handle) return;
  hideImgPop(); setStatus("saving");
  var markdown = editingPage.frontmatter + handle.getMarkdown() + "\n";
  fetch("/api/save-image", {method:"POST", headers:{"Content-Type":"application/json"},
    body: JSON.stringify({slug: editingPage.slug, url: url, markdown: markdown})})
    .then(function(r){ return r.json(); })
    .then(function(j){
      if (j.ok){ editingPage.body = j.body; dirty = false; window.onbeforeunload = null; markRow(false); editPage(editingPage); setStatus("saved"); }
      else { setStatus("error"); alert(j.error || "couldn't save image"); }
    })
    .catch(function(){ setStatus("error"); });
}
content.addEventListener("mouseover", function(e){
  var t = e.target;
  if (!t || t.tagName !== "IMG" || !t.closest("#editor")) return;
  if (diagramId(t) || isExternalImg(t.getAttribute("src") || "")) showImgActions(t);
});
content.addEventListener("mouseout", function(e){
  if (e.target && e.target.tagName === "IMG") hideImgPop();
});
addEventListener("scroll", function(){ if (imgPop && imgPop.style.display !== "none") imgPop.style.display = "none"; }, true);

// --- Excalidraw diagrams (lazy-loaded from the local /_excalidraw.js bundle) ---
function loadExcalidraw(cb){
  if (window.DocdExcalidraw) return cb();
  if (!document.getElementById("exc-css")){ var l=document.createElement("link"); l.id="exc-css"; l.rel="stylesheet"; l.href="/_excalidraw.css"; document.head.appendChild(l); }
  var s=document.createElement("script"); s.src="/_excalidraw.js";
  s.onload=function(){ cb(); };
  s.onerror=function(){ alert("Couldn't load the diagram editor."); };
  document.head.appendChild(s);
}
function openDiagram(initialData, onSave){
  loadExcalidraw(function(){
    var overlay=document.createElement("div"); overlay.className="exc-overlay";
    var bar=document.createElement("div"); bar.className="exc-bar";
    var title=document.createElement("strong"); title.className="exc-title"; title.textContent="Diagram";
    var spacer=document.createElement("span"); spacer.style.flex="1";
    var cancel=document.createElement("button"); cancel.className="btn"; cancel.textContent="Cancel";
    var save=document.createElement("button"); save.className="btn primary"; save.textContent="Save";
    var host=document.createElement("div"); host.className="exc-host";
    bar.append(title, spacer, cancel, save); overlay.append(bar, host);
    document.body.appendChild(overlay);
    var inst=window.DocdExcalidraw.open(host, initialData || undefined, { theme: currentTheme() });
    function close(){ try{ inst.destroy(); }catch(e){} overlay.remove(); }
    cancel.onclick=close;
    save.onclick=function(){
      save.disabled=true; save.textContent="Saving…";
      inst.exportSvg()
        .then(function(svg){ return onSave(inst.getScene(), svg); })
        .then(function(){ close(); })
        .catch(function(){ save.disabled=false; save.textContent="Save"; alert("Couldn't save diagram"); });
    };
  });
}
function insertDiagram(){
  if (!handle) return;
  openDiagram(null, function(scene, svg){
    var id = Date.now().toString(36) + Math.floor(Math.random()*1e6).toString(36);
    return fetch("/api/save-diagram", {method:"POST", headers:{"Content-Type":"application/json"},
      body: JSON.stringify({id:id, scene:scene, svg:svg})})
      .then(function(r){ return r.json(); })
      .then(function(j){ if(!j.ok) throw new Error(j.error||"save failed");
        if (handle.insertImage){ handle.insertImage("img/diagram-"+id+".svg", "excalidraw:"+id); onEdit(true); } });
  });
}
// Let the editor's "/diagram" slash item trigger the same insert flow.
window.caretInsertDiagram = insertDiagram;
function editDiagram(id){
  fetch("/diagrams/"+id+".excalidraw")
    .then(function(r){ return r.ok ? r.json() : null; })
    .then(function(scene){
      openDiagram(scene, function(newScene, svg){
        return fetch("/api/save-diagram", {method:"POST", headers:{"Content-Type":"application/json"},
          body: JSON.stringify({id:id, scene:newScene, svg:svg})})
          .then(function(r){ return r.json(); })
          .then(function(j){ if(!j.ok) throw new Error(j.error||"save failed");
            // The browser caches the decoded SVG by URL, so only a full reload shows
            // the new one. Persist any unsaved doc edits first, then reload.
            if (dirty && handle){
              var md = editingPage.frontmatter + handle.getMarkdown() + "\n";
              return fetch("/api/save", {method:"POST", headers:{"Content-Type":"application/json"},
                body: JSON.stringify({slug: editingPage.slug, markdown: md})})
                .then(function(){ location.reload(); });
            }
            location.reload();
          });
      });
    });
}
var dbtn=document.getElementById("diagrambtn"); if (dbtn) dbtn.addEventListener("click", insertDiagram);

// --- file management: create / rename / delete pages ---
var ICON_TRASH = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M6 6l1 14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-14"/></svg>';
function structuralReload(hash){ if (hash) location.hash = hash; location.reload(); }
function createPage(path){
  fetch("/api/create-page", {method:"POST", headers:{"Content-Type":"application/json"}, body: JSON.stringify({path: path})})
    .then(function(r){ return r.json(); })
    .then(function(j){ if (j.ok){ structuralReload("#/" + j.slug); } else { alert(j.error || "couldn't create page"); } })
    .catch(function(){ alert("couldn't create page"); });
}
function renamePage(slug){
  var to = prompt("Move/rename to (path, e.g. guide/setup):", slug);
  if (!to || to === slug) return;
  fetch("/api/rename-page", {method:"POST", headers:{"Content-Type":"application/json"}, body: JSON.stringify({from: slug, to: to})})
    .then(function(r){ return r.json(); })
    .then(function(j){ if (j.ok){ structuralReload("#/" + j.slug); } else { alert(j.error || "couldn't rename"); } })
    .catch(function(){ alert("couldn't rename"); });
}
function deletePage(slug){
  if (!confirm('Delete "' + slug + '"? This removes the file.')) return;
  fetch("/api/delete-page", {method:"POST", headers:{"Content-Type":"application/json"}, body: JSON.stringify({slug: slug})})
    .then(function(r){ return r.json(); })
    .then(function(j){ if (j.ok){ structuralReload("#/index"); } else { alert(j.error || "couldn't delete"); } })
    .catch(function(){ alert("couldn't delete"); });
}
(function(){
  var btn = document.getElementById("newpage"); if (!btn) return;
  btn.addEventListener("click", function(){
    var foot = btn.parentNode; foot.innerHTML = "";
    var box = document.createElement("div"); box.className = "navadd";
    var inp = document.createElement("input"); inp.placeholder = "guide/setup"; inp.spellcheck = false;
    box.appendChild(inp); foot.appendChild(box); inp.focus();
    function restore(){ foot.innerHTML = ""; foot.appendChild(btn); }
    inp.addEventListener("keydown", function(e){
      if (e.key === "Enter"){ var v = inp.value.trim(); if (v) createPage(v); else restore(); }
      else if (e.key === "Escape"){ restore(); }
    });
    inp.addEventListener("blur", restore);
  });
})();
var rowPop = null, rowHideT = 0;
function hideRowPop(){ rowHideT = setTimeout(function(){ if (rowPop) rowPop.style.display = "none"; }, 220); }
function ensureRowPop(){
  if (rowPop) return rowPop;
  rowPop = document.createElement("div"); rowPop.id = "rowactions";
  document.body.appendChild(rowPop);
  rowPop.addEventListener("mouseenter", function(){ clearTimeout(rowHideT); });
  rowPop.addEventListener("mouseleave", hideRowPop);
  return rowPop;
}
function showRowActions(row){
  var slug = row.dataset.slug; if (!slug) return;
  var p = ensureRowPop(); clearTimeout(rowHideT); p.innerHTML = "";
  p.appendChild(iconBtn(ICON_EDIT, "Rename / move", function(){ hideRowPop(); renamePage(slug); }));
  var del = iconBtn(ICON_TRASH, "Delete page", function(){ hideRowPop(); deletePage(slug); });
  del.className += " danger"; p.appendChild(del);
  p.style.display = "flex";
  var r = row.getBoundingClientRect();
  p.style.top = (r.top + (r.height - 28) / 2) + "px";
  p.style.left = Math.max(8, r.right - p.offsetWidth - 6) + "px";
}
(function(){
  var sb = document.getElementById("sidebar"); if (!sb) return;
  sb.addEventListener("mouseover", function(e){
    var row = e.target && e.target.closest ? e.target.closest(".navrow") : null;
    if (row) showRowActions(row);
  });
  sb.addEventListener("mouseout", function(e){
    if (e.target && e.target.closest && e.target.closest(".navrow")) hideRowPop();
  });
})();
"##;

const TEMPLATE: &str = r##"<!doctype html>
<html lang="en" data-theme="__THEME__">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>caret</title>
<style>
*{box-sizing:border-box}
:root{
  --canvas:#0B0B0C;--panel:#141416;--panel-2:#1A1A1D;--inset:#0F0F11;
  --fg:#ECECEE;--fg-2:#C7C7CC;--muted:#85858E;--faint:#56565E;
  --border:#262629;--border-2:#33333A;--hair:#1E1E21;
  --accent:#4C8DFF;--accent-ink:#0B1B36;--accent-soft:rgba(76,141,255,.12);
  --state-saved:#54B27D;--state-unsaved:#E0A458;--state-saving:#6E9BF0;--state-error:#E0675E;
  --sans:ui-sans-serif,system-ui,-apple-system,"Segoe UI",Helvetica,Arial,sans-serif;
  --mono:ui-monospace,"SF Mono","JetBrains Mono",Menlo,Consolas,monospace;
  --serif:"Iowan Old Style","Palatino Linotype",Palatino,Georgia,serif;
  --r-xs:5px;--r-sm:6px;--r-md:8px;--r-lg:10px;--r-xl:14px;
  --shadow-sm:0 1px 2px rgba(0,0,0,.25);
  --shadow-md:0 1px 2px rgba(0,0,0,.25),0 18px 50px -24px rgba(0,0,0,.5);
  --ease:cubic-bezier(.2,.6,.2,1);--dur-1:120ms;--dur-2:160ms;
  --page-w:8.5in;--page-px:1in;--page-py:.85in;
  --diagram-h:360px;
}
:root[data-theme="light"]{
  --canvas:#EBEBEC;--panel:#FFFFFF;--panel-2:#F6F6F7;--inset:#F1F1F2;
  --fg:#16161A;--fg-2:#3A3A40;--muted:#6B6B73;--faint:#9A9AA2;
  --border:#E6E6E8;--border-2:#D8D8DC;--hair:#EDEDEF;
  --accent:#2F6FE6;--accent-ink:#FFFFFF;--accent-soft:rgba(47,111,230,.10);
  --shadow-sm:0 1px 2px rgba(16,16,26,.06);
  --shadow-md:0 1px 2px rgba(16,16,26,.06),0 12px 32px -16px rgba(16,16,26,.18);
}
@keyframes breathe{0%,100%{opacity:.35;transform:scale(.85)}50%{opacity:1;transform:scale(1)}}
html,body{height:100%}
body{margin:0;background:var(--canvas);color:var(--fg-2);font-family:var(--sans);font-size:14px;line-height:1.5;display:grid;grid-template-columns:264px 1fr;overflow:hidden}
::selection{background:var(--accent-soft)}
a{color:var(--accent);text-decoration:none}
button{font:inherit}
:focus-visible{outline:none;box-shadow:0 0 0 2px var(--canvas),0 0 0 4px var(--accent);border-radius:var(--r-sm)}
.ProseMirror:focus,.ProseMirror:focus-visible{outline:none;box-shadow:none}
#sidebar{background:var(--panel);border-right:1px solid var(--border);height:100vh;overflow-y:auto;padding:14px 10px;display:flex;flex-direction:column;gap:2px}
.brand{display:flex;align-items:center;gap:9px;padding:6px 8px 14px}
.brand .wordmark{font-family:var(--mono);font-size:16px;font-weight:600;letter-spacing:-.03em;color:var(--fg)}
.navrow{display:flex;align-items:center;gap:8px;position:relative;padding:6px 10px;border-radius:var(--r-sm);color:var(--fg-2);font-size:12px;font-weight:500;line-height:1.4}
.navrow .ico{color:var(--muted);flex:0 0 auto}
.navrow .lbl{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.navrow:hover{background:var(--panel-2)}
.navrow.active{background:var(--accent-soft);color:var(--fg)}
.navrow.active .ico{color:var(--accent)}
.navrow.dirty::before{content:"";position:absolute;left:1px;top:50%;transform:translateY(-50%);width:6px;height:6px;border-radius:50%;background:var(--state-unsaved)}
main{display:flex;flex-direction:column;height:100vh;overflow:hidden}
#topbar{flex:0 0 auto;display:flex;align-items:center;gap:10px;height:48px;padding:0 18px;background:var(--panel);border-bottom:1px solid var(--hair)}
#doctitle{font-size:14px;font-weight:600;color:var(--fg);letter-spacing:-.01em}
.grow{flex:1}
.editor-only{display:none}
body.editable .editor-only{display:flex}
#status{align-items:center;gap:7px;font-size:12px;color:var(--muted);font-variant-numeric:tabular-nums}
#status.error{color:var(--state-error)}
.dot{width:8px;height:8px;border-radius:50%;display:inline-block;flex:0 0 auto}
.dot.saved{background:transparent;box-shadow:inset 0 0 0 1.5px var(--state-saved)}
.dot.unsaved{background:var(--state-unsaved)}
.dot.saving{background:var(--state-saving);animation:breathe 1.6s var(--ease) infinite}
.dot.error{background:var(--state-error)}
.btn{display:inline-flex;align-items:center;gap:7px;padding:5px 11px;border:1px solid var(--border-2);background:transparent;color:var(--fg-2);border-radius:var(--r-md);cursor:pointer;font-size:13px;transition:background var(--dur-1) var(--ease),color var(--dur-1) var(--ease)}
.btn:hover{background:var(--panel-2);color:var(--fg)}
.btn.ghost{border-color:transparent;color:var(--muted)}
.btn.ghost:hover{color:var(--fg);background:var(--panel-2)}
.btn.icon{padding:6px;color:var(--muted)}
.btn.icon:hover{color:var(--fg);background:var(--panel-2)}
kbd{font-family:var(--mono);font-size:11px;color:var(--faint);border:1px solid var(--border);border-radius:var(--r-xs);padding:1px 5px;background:var(--inset)}
body.autosave #savebtn{opacity:.45}
.switch{display:inline-flex;align-items:center;gap:8px;cursor:pointer;font-size:12px;color:var(--muted);user-select:none}
.switch input{position:absolute;opacity:0;width:0;height:0}
.switch .track{width:34px;height:20px;border-radius:999px;background:var(--inset);border:1px solid var(--border-2);position:relative;transition:background var(--dur-2) var(--ease)}
.switch .track::after{content:"";position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;background:var(--muted);transition:transform var(--dur-2) var(--ease),background var(--dur-2)}
.switch input:checked + .track{background:var(--accent);border-color:var(--accent)}
.switch input:checked + .track::after{transform:translateX(14px);background:var(--accent-ink)}
#content{flex:1;overflow-y:auto;padding:28px 24px}
#editor,.sheet{width:var(--page-w);max-width:100%;margin:0 auto;background:transparent}
.sheet{padding:var(--page-py) var(--page-px)}
.prose{max-width:none;color:var(--fg-2);font-family:var(--serif);font-size:16px;line-height:1.7}
.prose h1,.prose h2,.prose h3{font-family:var(--sans);color:var(--fg);text-wrap:pretty}
.prose h1{font-size:30px;line-height:1.15;letter-spacing:-.02em;font-weight:600;margin:0 0 .5em}
.prose h2{font-size:22px;line-height:1.2;letter-spacing:-.018em;font-weight:600;margin:1.4em 0 .4em}
.prose h3{font-size:17px;line-height:1.3;letter-spacing:-.01em;font-weight:600;margin:1.2em 0 .3em}
.prose p,.prose li{text-wrap:pretty}
.prose a{text-decoration:underline;text-decoration-color:var(--accent-soft);text-underline-offset:2px;cursor:pointer}
.prose code{font-family:var(--mono);font-size:.88em;background:var(--inset);border:1px solid var(--hair);border-radius:var(--r-xs);padding:1px 5px}
.prose pre{font-family:var(--mono);background:var(--inset);border:1px solid var(--border);border-radius:var(--r-lg);padding:14px 16px;overflow:auto;font-size:13px;line-height:1.7}
.prose pre code{background:none;border:0;padding:0}
.prose blockquote{margin:1em 0;padding-left:14px;border-left:3px solid var(--accent);color:var(--muted)}
.prose table{border-collapse:collapse;font-family:var(--sans);font-size:14px}
.prose td,.prose th{border:1px solid var(--border);padding:7px 11px}
.prose hr{border:0;border-top:1px solid var(--hair);margin:1.6em 0}
.prose img{max-width:100%;border:1px solid var(--border);border-radius:var(--r-lg)}
.empty{color:var(--muted)}
.loadfail{color:var(--muted);padding:40px 0;display:flex;gap:12px;align-items:center}
#book{display:none}
#imgactions,#rowactions{position:fixed;z-index:30;display:none;gap:3px;padding:3px;background:var(--panel);border:1px solid var(--border);border-radius:var(--r-md);box-shadow:var(--shadow-md)}
#imgactions .iconbtn,#rowactions .iconbtn{display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;padding:0;border:0;background:transparent;color:var(--muted);border-radius:var(--r-sm);cursor:pointer;transition:background var(--dur-1) var(--ease),color var(--dur-1) var(--ease)}
#imgactions .iconbtn:hover,#rowactions .iconbtn:hover{background:var(--panel-2);color:var(--fg)}
#rowactions .iconbtn.danger:hover{color:var(--state-error)}
#imgactions .iconbtn svg,#rowactions .iconbtn svg{width:16px;height:16px}
.lp-scrim{position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:60;display:flex;justify-content:center;align-items:flex-start}
.lp-box{margin-top:14vh;width:520px;max-width:92vw;background:var(--panel);border:1px solid var(--border);border-radius:var(--r-lg);box-shadow:var(--shadow-md);overflow:hidden;display:flex;flex-direction:column}
.lp-input{font:inherit;font-size:14px;padding:12px 16px;background:transparent;border:0;outline:none;color:var(--fg);border-bottom:1px solid var(--hair)}
.lp-input::placeholder{color:var(--faint)}
.lp-list{max-height:46vh;overflow-y:auto;padding:6px}
.lp-row{display:flex;align-items:baseline;gap:8px;padding:7px 10px;border-radius:var(--r-sm);cursor:pointer}
.lp-row.sel{background:var(--accent-soft)}
.lp-title{font-size:13px;color:var(--fg)}
.lp-slug{font-family:var(--mono);font-size:11px;color:var(--muted);margin-left:auto}
.navtree{display:flex;flex-direction:column;gap:2px}
.navfooter{margin-top:auto;padding:8px 4px 2px}
.navadd{display:flex;align-items:center;gap:6px;width:100%;padding:6px 10px;border:1px dashed var(--border-2);background:transparent;color:var(--muted);border-radius:var(--r-sm);cursor:pointer;font-size:12px;font:inherit;font-size:12px}
.navadd:hover{color:var(--fg);background:var(--panel-2)}
.navadd input{flex:1;background:transparent;border:0;outline:none;color:var(--fg);font:inherit;font-size:12px}
.exc-overlay{position:fixed;inset:0;z-index:50;background:var(--canvas);display:flex;flex-direction:column}
.exc-bar{flex:0 0 auto;display:flex;align-items:center;gap:8px;padding:10px 14px;background:var(--panel);border-bottom:1px solid var(--border)}
.exc-title{font-size:14px;color:var(--fg)}
.exc-host{flex:1;min-height:0;position:relative}
.exc-host .excalidraw{height:100%}
#editor .milkdown{
  --crepe-color-background:var(--canvas);--crepe-color-surface:var(--panel);--crepe-color-surface-low:var(--panel-2);
  --crepe-color-on-background:var(--fg-2);--crepe-color-on-surface:var(--fg);--crepe-color-on-surface-variant:var(--muted);
  --crepe-color-outline:var(--border);--crepe-color-primary:var(--accent);--crepe-color-secondary:var(--accent-soft);
  --crepe-color-on-secondary:var(--fg);--crepe-color-inverse:var(--fg);--crepe-color-on-inverse:var(--canvas);
  --crepe-color-inline-code:var(--fg);--crepe-color-inline-area:var(--inset);--crepe-color-error:var(--state-error);
  --crepe-color-hover:var(--panel-2);--crepe-color-selected:var(--accent-soft);
  --crepe-shadow-1:var(--shadow-sm);--crepe-shadow-2:var(--shadow-md);
  --crepe-font-default:var(--sans);--crepe-font-code:var(--mono);--crepe-font-title:var(--sans);
  background:transparent;color:var(--fg-2);min-height:60vh
}
#editor .ProseMirror{font-family:var(--serif);font-size:16px;line-height:1.7;padding:var(--page-py) var(--page-px);caret-color:var(--accent)}
#editor .ProseMirror h1,#editor .ProseMirror h2,#editor .ProseMirror h3{font-family:var(--sans);color:var(--fg)}
#editor a{cursor:pointer}
/* Frame diagrams to a consistent height regardless of the drawing's bounds. */
#editor img[src*="diagram-"],.prose img[src*="diagram-"]{height:var(--diagram-h);width:auto;max-width:100%;object-fit:contain;display:block;margin:10px auto;border:1px solid var(--border);border-radius:var(--r-lg);background:#fff;padding:8px}
/* Crepe defaults list markers, the selection toolbar, and the block handle to the
   faint --outline (border) token — which reads as disabled, especially in light
   mode. Give them real foreground colors; accent only when active. */
#editor .milkdown-list-item-block li .label-wrapper{color:var(--muted)}
#editor .milkdown-list-item-block li .label-wrapper svg{fill:var(--muted)}
#editor .milkdown-toolbar .toolbar-item svg{color:var(--fg-2);fill:var(--fg-2)}
#editor .milkdown-toolbar .toolbar-item:hover svg{color:var(--fg);fill:var(--fg)}
#editor .milkdown-toolbar .toolbar-item.active svg{color:var(--accent);fill:var(--accent)}
#editor .milkdown-toolbar .divider{background:var(--border)}
#editor .milkdown-block-handle .operation-item{opacity:1}
#editor .milkdown-block-handle .operation-item svg{fill:var(--canvas)}
/* custom caret: native is hidden only when our drawn caret is active (graceful fallback) */
#editor .milkdown.docd-has-caret .ProseMirror{caret-color:transparent}
.docd-caret{position:absolute;display:none;width:2px;border-radius:1px;background:var(--accent);transform:translateX(-1px);pointer-events:none;z-index:2;will-change:left,top,height;animation:caretblink 1.08s step-end infinite}
@keyframes caretblink{0%,50%{opacity:1}50.01%,100%{opacity:0}}
#scrim{position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:40}
.drawer{position:fixed;top:0;right:0;height:100vh;width:380px;max-width:92vw;background:var(--panel);border-left:1px solid var(--border);box-shadow:var(--shadow-md);z-index:41;display:flex;flex-direction:column}
.drawer[hidden],#scrim[hidden]{display:none}
#helpscrim{position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:55}
#help{position:fixed;z-index:56;top:50%;left:50%;transform:translate(-50%,-50%);width:680px;max-width:92vw;max-height:84vh;background:var(--panel);border:1px solid var(--border);border-radius:var(--r-xl);box-shadow:var(--shadow-md);display:flex;flex-direction:column}
#help[hidden],#helpscrim[hidden]{display:none}
.help-head{display:flex;align-items:center;justify-content:space-between;padding:14px 20px;border-bottom:1px solid var(--hair)}
.help-head strong{font-size:14px;color:var(--fg)}
.help-body{overflow-y:auto;padding:6px 26px 26px}
.help-body .prose{font-size:15px}
.drawer-head{display:flex;align-items:center;justify-content:space-between;padding:14px 16px;border-bottom:1px solid var(--hair)}
.drawer-head strong{font-size:14px;color:var(--fg)}
.drawer-body{padding:8px 16px 24px;overflow-y:auto}
.sec{font-family:var(--mono);font-size:11px;letter-spacing:.1em;text-transform:uppercase;color:var(--faint);margin:18px 0 4px}
.sec .hint{font-family:var(--sans);letter-spacing:0;text-transform:none;color:var(--faint);font-size:11px;margin-left:8px}
.row{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;padding:12px 0;border-top:1px solid var(--hair)}
.row .rt{font-size:13px;color:var(--fg);font-weight:500}
.row .rd{font-size:12px;color:var(--muted);margin-top:2px}
.seg{display:inline-flex;border:1px solid var(--border-2);border-radius:var(--r-md);overflow:hidden;flex:0 0 auto}
.seg-btn{padding:5px 12px;background:transparent;border:0;color:var(--muted);cursor:pointer;font-size:12px}
.seg-btn.active{background:var(--accent-soft);color:var(--fg)}
.keyrow{display:flex;gap:6px;align-items:center;flex:0 0 auto}
.inp{background:var(--inset);border:1px solid var(--border-2);border-radius:var(--r-md);color:var(--fg);padding:6px 9px;font-size:13px;font-family:var(--mono);width:150px}
.inp.wide{width:200px;font-family:var(--sans)}
.inp::placeholder{color:var(--faint)}
@media (prefers-reduced-motion:reduce){*{animation:none!important;transition:none!important}}
@media print{
  /* Remap every token to a printable light palette, regardless of theme, so all
     token-driven styling (prose + the editor bridge) prints black-on-white. */
  :root,:root[data-theme="light"],:root[data-theme="dark"]{
    --canvas:#fff;--panel:#fff;--panel-2:#fff;--inset:#f4f4f4;
    --fg:#000;--fg-2:#111;--muted:#444;--faint:#777;
    --border:#cfcfcf;--border-2:#bdbdbd;--hair:#e2e2e2;
    --accent:#1a4fb4;--accent-ink:#fff;--accent-soft:transparent;
  }
  @page{margin:1in}
  html,body{height:auto;overflow:visible;background:#fff;display:block}
  /* Ctrl/Cmd+P prints the whole repo as a book: hide the live app, show #book. */
  #sidebar,main,#scrim,#drawer{display:none!important}
  #book{display:block}
  #book .sheet{width:auto;max-width:none;margin:0;padding:0;background:#fff;border:0;box-shadow:none;border-radius:0;break-before:page}
  #book .sheet:first-child{break-before:auto}
  #book .prose{font-size:11.5pt;line-height:1.5;color:#000}
  #book .prose a{color:#000;text-decoration:underline}
  #book .prose h1,#book .prose h2,#book .prose h3{break-after:avoid}
  #book p,#book li,#book blockquote{orphans:3;widows:3}
  #book pre,#book blockquote,#book table,#book img,#book figure{break-inside:avoid}
}
</style>
__EDITOR_ASSETS__
</head>
<body>
<nav id="sidebar">
  <div class="brand">
    <svg viewBox="0 0 32 32" width="22" height="22" aria-hidden="true"><rect x="1" y="1" width="30" height="30" rx="8" fill="var(--accent)"/><path d="M8 21 L16 11 L24 21" fill="none" stroke="var(--accent-ink)" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round"/></svg>
    <span class="wordmark">caret</span>
  </div>
  <div class="navtree">__NAV__</div>
  <div class="navfooter editor-only"><button id="newpage" class="navadd" type="button">+ New page</button></div>
</nav>
<main>
  <header id="topbar">
    <strong id="doctitle"></strong>
    <span class="grow"></span>
    <span id="status" class="editor-only" aria-live="polite"><span id="statusdot" class="dot saved"></span><span id="statuslbl">Saved</span></span>
    <button id="savebtn" class="btn ghost editor-only">Save <kbd>&#8984;S</kbd></button>
    <label class="switch editor-only"><input type="checkbox" id="autosave"><span class="track"></span><span>Auto-save</span></label>
    <button id="diagrambtn" class="btn icon editor-only" aria-label="Insert diagram" title="Insert diagram"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><path d="M10 6.5h4a3 3 0 0 1 3 3v4"/></svg></button>
    <button id="helpbtn" class="btn icon editor-only" aria-label="Help" title="Help"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg></button>
    <button id="gear" class="btn icon editor-only" aria-label="Settings"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 8 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg></button>
  </header>
  <div id="content"></div>
</main>
<div id="book" aria-hidden="true"></div>
<div id="scrim" hidden></div>
<aside id="drawer" class="drawer" hidden aria-label="Settings">
  <div class="drawer-head"><strong>Settings</strong><button id="drawerclose" class="btn icon" aria-label="Close">&#10005;</button></div>
  <div class="drawer-body">
    <div class="sec">Workspace</div>
    <div class="row"><div><div class="rt">Project title</div><div class="rd">Shown in the header. Defaults to the folder name.</div></div><input type="text" id="set-title" class="inp wide"></div>
    <div class="row"><div><div class="rt">Auto-save</div><div class="rd">Persist edits automatically as you type.</div></div><label class="switch"><input type="checkbox" id="set-autosave"><span class="track"></span></label></div>
    <div class="row"><div><div class="rt">Theme</div><div class="rd">Default appearance for this repo.</div></div><div class="seg" id="themeseg"><button class="seg-btn" data-theme-val="dark">Dark</button><button class="seg-btn" data-theme-val="light">Light</button></div></div>
    <div class="sec">Secrets <span class="hint">.caret/secrets.toml &middot; gitignored, never committed</span></div>
    <div class="row"><div><div class="rt">Anthropic API key</div><div class="rd" id="anthropic-status">Not set</div></div><div class="keyrow"><input type="password" id="anthropic-key" class="inp" placeholder="sk-ant-&hellip;"><button id="anthropic-save" class="btn">Save</button></div></div>
  </div>
</aside>
<div id="helpscrim" hidden></div>
<aside id="help" hidden aria-label="Help">
  <div class="help-head"><strong>caret &mdash; guide</strong><button id="helpclose" class="btn icon" aria-label="Close">&#10005;</button></div>
  <div class="help-body"><article class="prose">__HELP_HTML__</article></div>
</aside>
<script>
const PAGES = __DATA__;
const EDITABLE = __EDITABLE__;
const SETTINGS = __SETTINGS__;
var AUTO_SAVE = !!SETTINGS.autoSave;
const bySlug = Object.fromEntries(PAGES.map(function(p){return [p.slug, p];}));
const content = document.getElementById("content");
var dirty = false, editingSlug = null, suppressGuard = false;
var STATUS_LABEL = {saved:"Saved", unsaved:"Unsaved", saving:"Saving", error:"Couldn't save"};
function setStatus(state){
  var dot = document.getElementById("statusdot"), lbl = document.getElementById("statuslbl"), wrap = document.getElementById("status");
  if (dot) dot.className = "dot " + state;
  if (lbl) lbl.textContent = STATUS_LABEL[state] || "";
  if (wrap) wrap.classList.toggle("error", state === "error");
}
function applyTheme(t){ document.documentElement.setAttribute("data-theme", t === "light" ? "light" : "dark"); }
function currentTheme(){ return document.documentElement.getAttribute("data-theme") || "dark"; }
function current(){
  var h = location.hash.replace(/^#\/?/, "");
  if (bySlug[h]) return h;
  if (bySlug["index"]) return "index";
  return PAGES.length ? PAGES[0].slug : "";
}
function render(){
  var slug = current(), page = bySlug[slug];
  document.title = (SETTINGS.project || "caret") + (page ? " · " + page.title : "");
  var links = document.querySelectorAll("#sidebar .navrow");
  for (var i=0;i<links.length;i++){ links[i].classList.toggle("active", links[i].dataset.slug === slug); }
  if (EDITABLE){ if (page){ editPage(page); } else { content.innerHTML = '<p class="empty">Not found.</p>'; } return; }
  content.innerHTML = page ? '<div class="sheet"><article class="prose">' + page.html + '</article></div>' : '<p class="empty">Not found.</p>';
  content.scrollTop = 0;
}
__EDITOR_JS__
function persistSettings(){
  return fetch("/api/settings",{method:"POST",headers:{"Content-Type":"application/json"},
    body:JSON.stringify({auto_save:AUTO_SAVE, theme:currentTheme(), title:(SETTINGS.title||"")})}).then(function(r){return r.json();}).catch(function(){});
}
function applyProjectTitle(){
  var name = ((SETTINGS.title||"").trim()) || (SETTINGS.basename||"");
  SETTINGS.project = name;
  var dt = document.getElementById("doctitle"); if (dt) dt.textContent = name;
  var page = bySlug[current()];
  document.title = name + (page ? " · " + page.title : "");
}
function syncAutoSaveUI(){
  var a=document.getElementById("autosave"), b=document.getElementById("set-autosave");
  if (a) a.checked = AUTO_SAVE; if (b) b.checked = AUTO_SAVE;
  document.body.classList.toggle("autosave", AUTO_SAVE);
}
function onToggleAutoSave(val){
  AUTO_SAVE = val; syncAutoSaveUI(); persistSettings();
  if (AUTO_SAVE && dirty && typeof scheduleAutoSave === "function") scheduleAutoSave();
}
function syncThemeSeg(){
  var t=currentTheme(), btns=document.querySelectorAll("#themeseg .seg-btn");
  for (var i=0;i<btns.length;i++){ btns[i].classList.toggle("active", btns[i].dataset.themeVal === t); }
}
function syncSecrets(){
  var has = (SETTINGS.secrets||[]).indexOf("anthropic") >= 0;
  var s=document.getElementById("anthropic-status"); if (s) s.textContent = has ? "Set" : "Not set";
}
function initChrome(){
  applyTheme(SETTINGS.theme);
  document.body.classList.toggle("editable", EDITABLE);
  applyProjectTitle();
  var ti=document.getElementById("set-title");
  if (ti){ ti.value = SETTINGS.title || ""; ti.placeholder = SETTINGS.basename || "Project title";
    ti.addEventListener("change", function(){ SETTINGS.title = ti.value; persistSettings(); applyProjectTitle(); }); }
  syncAutoSaveUI(); syncThemeSeg(); syncSecrets();
  var sb=document.getElementById("savebtn"); if (sb) sb.addEventListener("click", function(){ if (typeof saveCurrent==="function") saveCurrent(); });
  var a=document.getElementById("autosave"); if (a) a.addEventListener("change", function(){ onToggleAutoSave(a.checked); });
  var b=document.getElementById("set-autosave"); if (b) b.addEventListener("change", function(){ onToggleAutoSave(b.checked); });
  var gear=document.getElementById("gear"), drawer=document.getElementById("drawer"), scrim=document.getElementById("scrim"), dc=document.getElementById("drawerclose");
  function openD(){ if(drawer) drawer.hidden=false; if(scrim) scrim.hidden=false; }
  function closeD(){ if(drawer) drawer.hidden=true; if(scrim) scrim.hidden=true; }
  if (gear) gear.addEventListener("click", openD);
  if (dc) dc.addEventListener("click", closeD);
  if (scrim) scrim.addEventListener("click", closeD);
  var helpbtn=document.getElementById("helpbtn"), help=document.getElementById("help"), helpscrim=document.getElementById("helpscrim"), helpclose=document.getElementById("helpclose");
  function openHelp(){ if(help) help.hidden=false; if(helpscrim) helpscrim.hidden=false; }
  function closeHelp(){ if(help) help.hidden=true; if(helpscrim) helpscrim.hidden=true; }
  if (helpbtn) helpbtn.addEventListener("click", openHelp);
  if (helpclose) helpclose.addEventListener("click", closeHelp);
  if (helpscrim) helpscrim.addEventListener("click", closeHelp);
  addEventListener("keydown", function(e){
    if (e.key!=="Escape") return;
    if (drawer && !drawer.hidden) closeD();
    if (help && !help.hidden) closeHelp();
  });
  var seg=document.getElementById("themeseg");
  if (seg) seg.addEventListener("click", function(e){
    var btn = e.target.closest ? e.target.closest(".seg-btn") : null; if(!btn) return;
    applyTheme(btn.dataset.themeVal); syncThemeSeg(); persistSettings();
  });
  var ks=document.getElementById("anthropic-save");
  if (ks) ks.addEventListener("click", function(){
    var inp=document.getElementById("anthropic-key"); var val=inp?inp.value:"";
    fetch("/api/secret",{method:"POST",headers:{"Content-Type":"application/json"},
      body:JSON.stringify({name:"anthropic", value:val})}).then(function(r){return r.json();})
      .then(function(j){ if(j && j.secrets){ SETTINGS.secrets=j.secrets; } if(inp) inp.value=""; syncSecrets(); })
      .catch(function(){ alert("Couldn't save key"); });
  });
}
initChrome();
// Ctrl/Cmd+P prints the whole repo as a book. Built eagerly (hidden) so its images
// — including diagram SVGs — are already loaded when the print snapshot is taken.
function buildBook(){
  var book = document.getElementById("book"); if (!book) return;
  var h = "";
  for (var i=0;i<PAGES.length;i++){ h += '<section class="sheet"><article class="prose">' + PAGES[i].html + '</article></section>'; }
  book.innerHTML = h;
}
buildBook();
addEventListener("hashchange", function(){
  if (suppressGuard){ suppressGuard = false; return; }
  if (dirty){
    if (!confirm("Discard unsaved changes?")){ suppressGuard = true; location.hash = "#/" + editingSlug; return; }
    dirty = false; window.onbeforeunload = null;
  }
  render();
});
render();
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_resolve() {
        assert_eq!(
            slug_for("guide/getting-started.md"),
            "guide/getting-started"
        );
        assert_eq!(slug_for("index.md"), "index");
        assert_eq!(
            resolve_link("guide/getting-started", "concepts.md"),
            "#/guide/concepts"
        );
        assert_eq!(
            resolve_link("guide/getting-started", "../index.md"),
            "#/index"
        );
        assert_eq!(resolve_link("index", "architecture.md"), "#/architecture");
        assert_eq!(resolve_link("index", "https://x.com"), "https://x.com");
        assert_eq!(resolve_link("index", "#/foo"), "#/foo");
        assert_eq!(resolve_link("index", "page.md#section"), "#/page");
    }

    #[test]
    fn frontmatter_parses() {
        let (fm, body) = split_frontmatter("---\ntitle: Home\norder: 2\n---\n\n# Hi\n");
        assert_eq!(frontmatter_meta(&fm), (Some("Home".to_string()), 2));
        assert!(body.contains("# Hi"));
    }

    #[test]
    fn no_frontmatter() {
        let (fm, body) = split_frontmatter("# Just a heading\n");
        assert_eq!(fm, "");
        assert_eq!(frontmatter_meta(&fm), (None, 999));
        assert_eq!(first_h1(&body), Some("Just a heading".to_string()));
    }

    #[test]
    fn split_frontmatter_roundtrips() {
        let raw = "---\ntitle: T\norder: 1\n---\n\n# H\n\nbody\n";
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(
            format!("{fm}{body}"),
            raw,
            "split must reconstruct the file"
        );
        assert!(fm.starts_with("---\n") && fm.trim_end().ends_with("---"));
        assert!(body.contains("# H"));

        let (fm2, body2) = split_frontmatter("# Just body\n");
        assert_eq!(fm2, "");
        assert_eq!(body2, "# Just body\n");
    }

    #[test]
    fn no_end_list_artifact() {
        // adjacent lists previously left a literal `<!-- end list -->` comment
        let n = normalize_markdown("1. a\n2. b\n\n- x\n- y\n");
        assert!(
            !n.contains("end list"),
            "end-list comment must be stripped:\n{n}"
        );
        assert!(n.contains("1.") && n.contains("- x"));
        // stripping is stable
        assert_eq!(n, normalize_markdown(&n));
    }

    #[test]
    fn normalize_is_idempotent() {
        // Once a file is normalized, later edits diff only the changed lines.
        let raw = "---\ntitle: T\norder: 1\n---\n\n# Heading\n\n- a\n- b\n\nText with **bold**.\n";
        let once = normalize_markdown(raw);
        let twice = normalize_markdown(&once);
        assert_eq!(once, twice, "normalize must be a fixed point");
        assert!(once.contains("title: T"), "frontmatter must survive");
    }
}
