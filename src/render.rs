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

/// Split YAML-ish frontmatter. Returns (title, order, body).
/// Minimal `key: value` parser; swap for full YAML if frontmatter grows.
pub fn parse_doc(raw: &str) -> (Option<String>, i64, String) {
    if let Some(after_open) = raw.strip_prefix("---\n") {
        if let Some(idx) = after_open.find("\n---") {
            let fm = &after_open[..idx];
            let rest = &after_open[idx + 4..];
            let body = rest.find('\n').map(|n| &rest[n + 1..]).unwrap_or("");
            let mut title = None;
            let mut order = 999;
            for line in fm.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    match k.trim() {
                        "title" => title = Some(v.to_string()),
                        "order" => order = v.parse().unwrap_or(999),
                        _ => {}
                    }
                }
            }
            return (title, order, body.to_string());
        }
    }
    (None, 999, raw.to_string())
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
    String::from_utf8(out).expect("utf8")
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
        let (fm_title, order, _) = parse_doc(&raw);
        let (frontmatter, body) = split_frontmatter(&raw);
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
        .replace("__EDITABLE__", if editable { "true" } else { "false" })
        .replace(
            "__EDITOR_ASSETS__",
            if editable { editor_assets() } else { "" },
        )
        .replace("__EDITOR_JS__", if editable { EDITOR_JS } else { "" })
}

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
      if (j.ok){ editingPage.body = handle.getMarkdown(); handle.markSaved(); dirty = false; window.onbeforeunload = null; markRow(false); setStatus("saved"); }
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
});
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
/* custom caret: native is hidden only when our drawn caret is active (graceful fallback) */
#editor .milkdown.docd-has-caret .ProseMirror{caret-color:transparent}
.docd-caret{position:absolute;display:none;width:2px;border-radius:1px;background:var(--accent);transform:translateX(-1px);pointer-events:none;z-index:2;will-change:left,top,height;animation:caretblink 1.08s step-end infinite}
@keyframes caretblink{0%,50%{opacity:1}50.01%,100%{opacity:0}}
#scrim{position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:40}
.drawer{position:fixed;top:0;right:0;height:100vh;width:380px;max-width:92vw;background:var(--panel);border-left:1px solid var(--border);box-shadow:var(--shadow-md);z-index:41;display:flex;flex-direction:column}
.drawer[hidden],#scrim[hidden]{display:none}
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
  __NAV__
</nav>
<main>
  <header id="topbar">
    <strong id="doctitle"></strong>
    <span class="grow"></span>
    <span id="status" class="editor-only" aria-live="polite"><span id="statusdot" class="dot saved"></span><span id="statuslbl">Saved</span></span>
    <button id="savebtn" class="btn ghost editor-only">Save <kbd>&#8984;S</kbd></button>
    <label class="switch editor-only"><input type="checkbox" id="autosave"><span class="track"></span><span>Auto-save</span></label>
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
    <div class="sec">Secrets <span class="hint">.docd/secrets.toml &middot; gitignored, never committed</span></div>
    <div class="row"><div><div class="rt">Anthropic API key</div><div class="rd" id="anthropic-status">Not set</div></div><div class="keyrow"><input type="password" id="anthropic-key" class="inp" placeholder="sk-ant-&hellip;"><button id="anthropic-save" class="btn">Save</button></div></div>
  </div>
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
  addEventListener("keydown", function(e){ if (e.key==="Escape" && drawer && !drawer.hidden) closeD(); });
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
// Ctrl/Cmd+P prints the whole repo as a book — build the page sheets on demand.
function buildBook(){
  var book = document.getElementById("book"); if (!book) return;
  var h = "";
  for (var i=0;i<PAGES.length;i++){ h += '<section class="sheet"><article class="prose">' + PAGES[i].html + '</article></section>'; }
  book.innerHTML = h;
}
addEventListener("beforeprint", buildBook);
addEventListener("afterprint", function(){ var b = document.getElementById("book"); if (b) b.innerHTML = ""; });
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
        let (t, o, body) = parse_doc("---\ntitle: Home\norder: 2\n---\n\n# Hi\n");
        assert_eq!(t, Some("Home".to_string()));
        assert_eq!(o, 2);
        assert!(body.contains("# Hi"));
    }

    #[test]
    fn no_frontmatter() {
        let (t, o, body) = parse_doc("# Just a heading\n");
        assert_eq!(t, None);
        assert_eq!(o, 999);
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
    fn normalize_is_idempotent() {
        // Once a file is normalized, later edits diff only the changed lines.
        let raw = "---\ntitle: T\norder: 1\n---\n\n# Heading\n\n- a\n- b\n\nText with **bold**.\n";
        let once = normalize_markdown(raw);
        let twice = normalize_markdown(&once);
        assert_eq!(once, twice, "normalize must be a fixed point");
        assert!(once.contains("title: T"), "frontmatter must survive");
    }
}
