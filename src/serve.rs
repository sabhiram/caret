use crate::render;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tiny_http::{Header, Method, Request, Response, Server};

#[derive(Deserialize)]
struct SaveIn {
    slug: String,
    markdown: String,
}

pub fn serve(dir: &Path, port: u16) -> Result<()> {
    let server = Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("{e}"))?;
    println!("docd serving {} at http://127.0.0.1:{port}", dir.display());
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
    let html = render::build_html(&pages, true);
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
    req.respond(Response::from_string(html).with_header(header))?;
    Ok(())
}

fn handle_save(mut req: Request, dir: &Path) {
    let result = (|| -> Result<String> {
        let mut body = String::new();
        req.as_reader().read_to_string(&mut body)?;
        let input: SaveIn = serde_json::from_str(&body)?;
        let path = safe_path(dir, &input.slug)?;
        std::fs::write(&path, render::normalize_markdown(&input.markdown))?;
        Ok(input.slug)
    })();
    let (code, json) = match result {
        Ok(slug) => {
            println!("saved: {slug}");
            (200u16, "{\"ok\":true}".to_string())
        }
        Err(e) => (
            400,
            format!(
                "{{\"ok\":false,\"error\":{}}}",
                serde_json::to_string(&e.to_string()).unwrap()
            ),
        ),
    };
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let _ = req.respond(
        Response::from_string(json)
            .with_status_code(code)
            .with_header(header),
    );
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
