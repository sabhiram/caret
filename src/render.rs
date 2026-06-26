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
    /// Raw file contents (incl. frontmatter) — what the in-page editor edits.
    pub source: String,
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
        let (fm_title, order, body) = parse_doc(&raw);
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
            source: raw,
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

pub fn build_html(pages: &[Page], editable: bool) -> String {
    // Escape `<` so embedded page HTML can't break out of the <script> tag.
    let data = serde_json::to_string(pages)
        .unwrap()
        .replace('<', "\\u003c");
    let nav = pages
        .iter()
        .map(|p| {
            let depth = p.slug.matches('/').count();
            format!(
                r##"<a href="#/{slug}" data-slug="{slug}" style="padding-left:{pad}px">{title}</a>"##,
                slug = esc_attr(&p.slug),
                pad = 12 + depth * 16,
                title = esc_html(&p.title),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    TEMPLATE
        .replace("__NAV__", &nav)
        .replace("__DATA__", &data)
        .replace("__EDITABLE__", if editable { "true" } else { "false" })
}

const TEMPLATE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>docd</title>
<style>
:root{--fg:#1a1a1a;--muted:#666;--line:#e5e5e5;--accent:#2563eb;--sidebar:#fafafa;}
*{box-sizing:border-box}
body{margin:0;font:16px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;color:var(--fg);display:flex;min-height:100vh}
#sidebar{width:260px;flex:0 0 260px;background:var(--sidebar);border-right:1px solid var(--line);padding:20px 0;overflow-y:auto;height:100vh;position:sticky;top:0}
#sidebar .brand{font-weight:700;padding:0 16px 14px;font-size:18px}
#sidebar a{display:block;padding:6px 16px;color:var(--fg);text-decoration:none;font-size:14px;border-left:2px solid transparent}
#sidebar a:hover{background:#f0f0f0}
#sidebar a.active{color:var(--accent);border-left-color:var(--accent);font-weight:600}
main{flex:1;max-width:820px;margin:0 auto;padding:48px 40px;width:100%}
main h1{margin-top:0;line-height:1.25}
main h2,main h3{line-height:1.25}
main pre{background:#f6f8fa;padding:12px 16px;border-radius:6px;overflow:auto}
main code{background:#f6f8fa;padding:2px 5px;border-radius:4px;font-size:.9em}
main pre code{padding:0;background:none}
main a{color:var(--accent)}
main table{border-collapse:collapse}
main td,main th{border:1px solid var(--line);padding:6px 10px}
.bar{display:flex;gap:8px;margin-bottom:20px}
button{font:inherit;padding:6px 14px;border:1px solid var(--line);background:#fff;border-radius:6px;cursor:pointer}
button:hover{background:#f0f0f0}
button.primary{background:var(--accent);color:#fff;border-color:var(--accent)}
button.primary:hover{filter:brightness(1.05)}
textarea.editor{width:100%;min-height:62vh;font:14px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace;padding:14px;border:1px solid var(--line);border-radius:8px;resize:vertical}
</style>
</head>
<body>
<nav id="sidebar"><div class="brand">📄 docd</div>__NAV__</nav>
<main id="content"></main>
<script>
const PAGES = __DATA__;
const EDITABLE = __EDITABLE__;
const bySlug = Object.fromEntries(PAGES.map(function(p){return [p.slug, p];}));
const content = document.getElementById("content");
function current(){
  var h = location.hash.replace(/^#\/?/, "");
  if (bySlug[h]) return h;
  if (bySlug["index"]) return "index";
  return PAGES.length ? PAGES[0].slug : "";
}
function render(){
  var slug = current();
  var page = bySlug[slug];
  var bar = (EDITABLE && page) ? '<div class="bar"><button id="editbtn">✎ Edit</button></div>' : '';
  content.innerHTML = bar + (page ? page.html : "<p>Not found.</p>");
  if (EDITABLE && page){ document.getElementById("editbtn").onclick = function(){ edit(page); }; }
  document.title = page ? page.title + " · docd" : "docd";
  var links = document.querySelectorAll("#sidebar a");
  for (var i=0;i<links.length;i++){ links[i].classList.toggle("active", links[i].dataset.slug === slug); }
  window.scrollTo(0,0);
}
function edit(page){
  content.innerHTML = "";
  var ta = document.createElement("textarea");
  ta.className = "editor"; ta.value = page.source;
  var bar = document.createElement("div"); bar.className = "bar";
  var save = document.createElement("button"); save.textContent = "Save"; save.className = "primary";
  var cancel = document.createElement("button"); cancel.textContent = "Cancel";
  save.onclick = function(){
    save.disabled = true; save.textContent = "Saving…";
    fetch("/api/save", {method:"POST", headers:{"Content-Type":"application/json"},
      body: JSON.stringify({slug: page.slug, markdown: ta.value})})
      .then(function(r){ return r.json(); })
      .then(function(j){
        if (j.ok){ location.reload(); }
        else { alert(j.error || "save failed"); save.disabled = false; save.textContent = "Save"; }
      })
      .catch(function(){ alert("save failed"); save.disabled = false; save.textContent = "Save"; });
  };
  cancel.onclick = render;
  bar.append(save, cancel);
  content.append(bar, ta);
  ta.focus();
}
addEventListener("hashchange", render);
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
    fn normalize_is_idempotent() {
        // Once a file is normalized, later edits diff only the changed lines.
        let raw = "---\ntitle: T\norder: 1\n---\n\n# Heading\n\n- a\n- b\n\nText with **bold**.\n";
        let once = normalize_markdown(raw);
        let twice = normalize_markdown(&once);
        assert_eq!(once, twice, "normalize must be a fixed point");
        assert!(once.contains("title: T"), "frontmatter must survive");
    }
}
