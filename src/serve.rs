use crate::{config, render};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tiny_http::{Header, Method, Request, Response, Server};

#[derive(Deserialize)]
struct SaveIn {
    slug: String,
    markdown: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct SettingsIn {
    auto_save: bool,
    theme: String,
    title: String,
}

impl Default for SettingsIn {
    fn default() -> Self {
        Self {
            auto_save: false,
            theme: "dark".into(),
            title: String::new(),
        }
    }
}

#[derive(Deserialize)]
struct SecretIn {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct SaveImageIn {
    slug: String,
    /// The current image src (an external http(s) URL) to persist.
    url: String,
    /// The doc's current full markdown (incl. frontmatter) to rewrite + save.
    markdown: String,
}

#[derive(Deserialize)]
struct SaveDiagramIn {
    id: String,
    /// The Excalidraw scene JSON (the editable source -> diagrams/<id>.excalidraw).
    scene: serde_json::Value,
    /// The exported SVG (what the doc renders -> img/diagram-<id>.svg).
    svg: String,
}

#[derive(Deserialize)]
struct CreatePageIn {
    path: String,
}
#[derive(Deserialize)]
struct DeletePageIn {
    slug: String,
}
#[derive(Deserialize)]
struct RenamePageIn {
    from: String,
    to: String,
}

// The Excalidraw editor bundle, vendored + served locally (lazy-loaded only when a
// diagram is opened). Huge, but never inlined into the page or the `build` output.
const EXCALIDRAW_JS: &str = include_str!("../assets/excalidraw.bundle.js");
const EXCALIDRAW_CSS: &str = include_str!("../assets/excalidraw.bundle.css");

pub fn serve(dir: &Path, port: u16) -> Result<()> {
    let server = Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("{e}"))?;
    println!("caret serving {} at http://127.0.0.1:{port}", dir.display());
    println!("open it, click Edit, and saves write straight back to the .md files.");
    for req in server.incoming_requests() {
        let method = req.method().clone();
        let url = req.url().to_string();
        match (method, url.as_str()) {
            (Method::Get, "/") | (Method::Get, "/index.html") => {
                if let Err(e) = serve_index(req, dir) {
                    eprintln!("render error: {e}");
                }
            }
            (Method::Post, "/api/save") => handle_save(req, dir),
            (Method::Get, "/api/settings") => respond_json(req, 200, &config::client_blob(dir).1),
            (Method::Post, "/api/settings") => handle_settings(req, dir),
            (Method::Post, "/api/secret") => handle_secret(req, dir),
            (Method::Post, "/api/save-image") => handle_save_image(req, dir),
            (Method::Post, "/api/save-diagram") => handle_save_diagram(req, dir),
            (Method::Post, "/api/create-page") => handle_create_page(req, dir),
            (Method::Post, "/api/delete-page") => handle_delete_page(req, dir),
            (Method::Post, "/api/rename-page") => handle_rename_page(req, dir),
            (Method::Get, "/_excalidraw.js") => {
                respond_asset(req, EXCALIDRAW_JS, "text/javascript; charset=utf-8")
            }
            (Method::Get, "/_excalidraw.css") => {
                respond_asset(req, EXCALIDRAW_CSS, "text/css; charset=utf-8")
            }
            // Any other GET serves a file from the repo (images, diagrams, etc.).
            (Method::Get, p) => serve_static(req, dir, p),
            _ => {
                let _ = req.respond(Response::from_string("not found").with_status_code(404));
            }
        }
    }
    Ok(())
}

fn serve_index(req: Request, dir: &Path) -> Result<()> {
    // Re-read on every load so the page reflects whatever is on disk right now.
    let pages = render::collect_pages(dir)?;
    let (theme, settings_json) = config::client_blob(dir);
    let html = render::build_html(&pages, true, &theme, &settings_json);
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
    req.respond(Response::from_string(html).with_header(header))?;
    Ok(())
}

fn read_body(req: &mut Request) -> Result<String> {
    let mut body = String::new();
    req.as_reader().read_to_string(&mut body)?;
    Ok(body)
}

fn respond_json(req: Request, code: u16, json: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let _ = req.respond(
        Response::from_string(json)
            .with_status_code(code)
            .with_header(header),
    );
}

/// Run a POST handler that reads the body and returns the success JSON; respond
/// 200 with it, or 400 with the error.
fn respond_result(mut req: Request, work: impl FnOnce(&mut Request) -> Result<String>) {
    match work(&mut req) {
        Ok(json) => respond_json(req, 200, &json),
        Err(e) => respond_json(req, 400, &err_json(&e)),
    }
}

fn handle_settings(req: Request, dir: &Path) {
    respond_result(req, |req| {
        let input: SettingsIn = serde_json::from_str(&read_body(req)?)?;
        let theme = if input.theme == "light" {
            "light"
        } else {
            "dark"
        };
        config::save(
            dir,
            &config::Settings {
                auto_save: input.auto_save,
                theme: theme.to_string(),
                title: input.title.trim().to_string(),
            },
        )?;
        Ok(config::client_blob(dir).1)
    });
}

fn handle_secret(req: Request, dir: &Path) {
    respond_result(req, |req| {
        let input: SecretIn = serde_json::from_str(&read_body(req)?)?;
        if input.name.trim().is_empty() {
            anyhow::bail!("missing secret name");
        }
        config::set_secret(dir, input.name.trim(), input.value.trim())?;
        Ok(config::client_blob(dir).1)
    });
}

fn err_json(e: &anyhow::Error) -> String {
    format!(
        "{{\"ok\":false,\"error\":{}}}",
        serde_json::to_string(&e.to_string()).unwrap()
    )
}

fn respond_asset(req: Request, body: &str, content_type: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap();
    let _ = req.respond(Response::from_string(body).with_header(header));
}

/// Persist an Excalidraw diagram: the scene JSON (the editable source) as a sidecar
/// `diagrams/<id>.excalidraw`, and the exported SVG as `img/diagram-<id>.svg` (what
/// the markdown renders). Editing a diagram never touches the `.md` — clean diffs.
fn handle_save_diagram(req: Request, dir: &Path) {
    respond_result(req, |req| {
        let input: SaveDiagramIn = serde_json::from_str(&read_body(req)?)?;
        let id = sanitize_id(&input.id)?;
        let diagrams = dir.join("diagrams");
        let img = dir.join("img");
        std::fs::create_dir_all(&diagrams)?;
        std::fs::create_dir_all(&img)?;
        std::fs::write(
            diagrams.join(format!("{id}.excalidraw")),
            serde_json::to_string_pretty(&input.scene)?,
        )?;
        std::fs::write(img.join(format!("diagram-{id}.svg")), input.svg.as_bytes())?;
        Ok("{\"ok\":true}".into())
    });
}

/// Keep a client-supplied diagram id to a safe filename fragment (no traversal).
fn sanitize_id(id: &str) -> Result<String> {
    let clean: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    if clean.is_empty() {
        anyhow::bail!("invalid diagram id");
    }
    Ok(clean)
}

/// Sanitize a user-supplied page path into (relative `.md` path, slug). Rejects
/// traversal/absolute paths; tolerates a `.md` extension or not.
fn sanitize_md_rel(path: &str) -> Result<(String, String)> {
    let p = path.trim().trim_start_matches('/').trim();
    let stem = p
        .strip_suffix(".md")
        .or_else(|| p.strip_suffix(".MD"))
        .unwrap_or(p)
        .trim_matches('/');
    if stem.is_empty()
        || stem
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        anyhow::bail!("invalid page path");
    }
    Ok((format!("{stem}.md"), stem.to_string()))
}

fn titleize(stem: &str) -> String {
    stem.rsplit('/')
        .next()
        .unwrap_or(stem)
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Remove now-empty parent directories of `file`, up to (but not including) `dir`.
fn remove_empty_parents(dir: &Path, file: &Path) {
    let base = match dir.canonicalize() {
        Ok(b) => b,
        Err(_) => return,
    };
    let mut cur = file.parent().map(|p| p.to_path_buf());
    while let Some(d) = cur {
        match d.canonicalize() {
            Ok(cd) if cd != base && cd.starts_with(&base) => {
                if std::fs::read_dir(&cd)
                    .map(|mut r| r.next().is_none())
                    .unwrap_or(false)
                {
                    let _ = std::fs::remove_dir(&cd);
                    cur = cd.parent().map(|p| p.to_path_buf());
                    continue;
                }
            }
            _ => {}
        }
        break;
    }
}

fn handle_create_page(req: Request, dir: &Path) {
    respond_result(req, |req| {
        let input: CreatePageIn = serde_json::from_str(&read_body(req)?)?;
        let (rel, slug) = sanitize_md_rel(&input.path)?;
        let path = dir.join(&rel);
        if path.exists() {
            anyhow::bail!("a page already exists at {slug}");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let title = titleize(&slug);
        let content = format!("---\ntitle: {title}\n---\n\n# {title}\n\n");
        std::fs::write(&path, render::normalize_markdown(&content))?;
        println!("created: {slug}");
        Ok(serde_json::json!({"ok": true, "slug": slug}).to_string())
    });
}

fn handle_delete_page(req: Request, dir: &Path) {
    respond_result(req, |req| {
        let input: DeletePageIn = serde_json::from_str(&read_body(req)?)?;
        let path = safe_path(dir, &input.slug)?;
        std::fs::remove_file(&path)?;
        remove_empty_parents(dir, &path);
        render::gc_orphan_diagrams(dir);
        println!("deleted: {}", input.slug);
        Ok("{\"ok\":true}".into())
    });
}

fn handle_rename_page(req: Request, dir: &Path) {
    respond_result(req, |req| {
        let input: RenamePageIn = serde_json::from_str(&read_body(req)?)?;
        let from = safe_path(dir, &input.from)?;
        let (rel, slug) = sanitize_md_rel(&input.to)?;
        let to = dir.join(&rel);
        if to.exists() {
            anyhow::bail!("a page already exists at {slug}");
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&from, &to)?;
        remove_empty_parents(dir, &from);
        render::gc_orphan_diagrams(dir);
        println!("renamed: {} -> {slug}", input.from);
        Ok(serde_json::json!({"ok": true, "slug": slug}).to_string())
    });
}

/// Download a referenced (external) image into the repo's `img/` dir, rewrite the
/// doc's markdown to point at the local copy, save it, and return the new body.
fn handle_save_image(req: Request, dir: &Path) {
    respond_result(req, |req| {
        let input: SaveImageIn = serde_json::from_str(&read_body(req)?)?;
        let doc_path = safe_path(dir, &input.slug)?;
        let name = image_filename(&input.url);
        let img_dir = dir.join("img");
        std::fs::create_dir_all(&img_dir)?;
        download_image(&input.url, &img_dir.join(&name))?;
        // Root-relative path; the SPA/build render at the repo root so this resolves
        // from every doc regardless of depth (and `serve` serves /img/* statically).
        let local = format!("img/{name}");
        let normalized = render::normalize_markdown(&input.markdown.replace(&input.url, &local));
        std::fs::write(&doc_path, &normalized)?;
        let body = render::split_frontmatter(&normalized).1;
        Ok(serde_json::json!({"ok": true, "body": body}).to_string())
    });
}

/// A unique, stable, sanitized filename for a downloaded image: a hash of the URL
/// (idempotent — same URL → same file) plus the URL's basename.
fn image_filename(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    let base = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or("image");
    let mut clean: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if clean.is_empty() || clean == "." {
        clean = "image".into();
    }
    if !clean.contains('.') {
        clean.push_str(".png"); // best-effort when the URL has no extension
    }
    format!("{:016x}-{clean}", h.finish())
}

/// Download an http(s) image via `curl` (keeps the binary free of TLS crates).
fn download_image(url: &str, dest: &Path) -> Result<()> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        anyhow::bail!("only http(s) images can be saved");
    }
    let status = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--proto",
            "=http,https",
            "--max-time",
            "30",
            "--max-filesize",
            "26214400", // 25 MiB cap
            "-o",
        ])
        .arg(dest)
        .arg(url)
        .status()
        .map_err(|e| anyhow!("curl unavailable: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(dest);
        anyhow::bail!("image download failed");
    }
    Ok(())
}

/// Serve a file from inside the repo (path-safe), for images and other assets.
fn serve_static(req: Request, dir: &Path, url_path: &str) {
    let rel = url_path.split(['?', '#']).next().unwrap_or(url_path);
    let rel = rel.trim_start_matches('/');
    let not_found =
        |req: Request| drop(req.respond(Response::from_string("not found").with_status_code(404)));
    if rel.is_empty() || rel.split('/').any(|s| s == "..") {
        return not_found(req);
    }
    let (base, path) = match (dir.canonicalize(), dir.join(rel).canonicalize()) {
        (Ok(b), Ok(p)) => (b, p),
        _ => return not_found(req),
    };
    if !path.starts_with(&base) || !path.is_file() {
        return not_found(req);
    }
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct =
                Header::from_bytes(&b"Content-Type"[..], content_type(&path).as_bytes()).unwrap();
            // no-store so an edited diagram SVG re-fetches fresh on re-mount.
            let nocache = Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap();
            let _ = req.respond(
                Response::from_data(bytes)
                    .with_header(ct)
                    .with_header(nocache),
            );
        }
        Err(_) => not_found(req),
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("json") | Some("excalidraw") => "application/json",
        _ => "application/octet-stream",
    }
}

fn handle_save(req: Request, dir: &Path) {
    // Return the freshly-rendered HTML so the client can refresh the print book
    // (and reading view) without a full reload.
    respond_result(req, |req| {
        let input: SaveIn = serde_json::from_str(&read_body(req)?)?;
        let path = safe_path(dir, &input.slug)?;
        let normalized = render::normalize_markdown(&input.markdown);
        std::fs::write(&path, &normalized)?;
        println!("saved: {}", input.slug);
        // Reconcile diagram files: drop any not referenced by a saved doc (incl. one
        // the user just deleted from a doc).
        let n = render::gc_orphan_diagrams(dir);
        if n > 0 {
            println!("gc: removed {n} orphaned diagram file(s)");
        }
        let html = render::render_html(&normalized, &input.slug);
        Ok(serde_json::json!({"ok": true, "html": html}).to_string())
    });
}

/// Map a slug to an EXISTING `.md` inside `dir`, rejecting path traversal.
/// This is the trust boundary: a browser POST must never write outside the project.
fn safe_path(dir: &Path, slug: &str) -> Result<PathBuf> {
    if slug.is_empty()
        || slug.starts_with('/')
        || slug
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        anyhow::bail!("invalid slug");
    }
    let base = dir.canonicalize()?;
    let canon = base
        .join(format!("{slug}.md"))
        .canonicalize()
        .map_err(|_| anyhow!("no such page: {slug}"))?;
    if !canon.starts_with(&base) {
        anyhow::bail!("path escapes project");
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_filename_is_safe_and_stable() {
        let a = image_filename("https://ex.com/a/photo.png?v=2");
        // idempotent for the same URL
        assert_eq!(a, image_filename("https://ex.com/a/photo.png?v=2"));
        assert!(a.ends_with("-photo.png"));
        // different URLs don't collide even with the same basename
        assert_ne!(a, image_filename("https://other.com/x/photo.png"));
        // no path traversal / unsafe chars survive
        let weird = image_filename("https://ex.com/../../etc/pa ss?wd");
        assert!(!weird.contains('/') && !weird.contains("..") && !weird.contains(' '));
        // extensionless URL gets a best-effort extension
        assert!(image_filename("https://ex.com/avatar").contains('.'));
    }
}
