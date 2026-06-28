mod config;
mod render;
mod serve;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "caret",
    version,
    about = "Render a directory of markdown into one interactive HTML page."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scaffold a new caret project (non-destructive; skips existing files)
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Build the single-page HTML from the markdown tree
    Build {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Output file (default: <dir>/index.html)
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Serve the docs with live in-page editing (saves write back to the .md files)
    Serve {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(short, long, default_value_t = 4321)]
        port: u16,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Init { dir } => init(&dir),
        Cmd::Build { dir, out } => build(&dir, out),
        Cmd::Serve { dir, port } => serve::serve(&dir, port),
    }
}

fn init(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let p = dir.join("index.md");
    if p.exists() {
        println!("skip (exists): index.md");
    } else {
        let note = "> **New here?** This page is just a guide. When you're ready, \
            delete it (hover it in the sidebar and hit the trash icon) and start \
            adding your own pages with **+ New page** at the bottom of the sidebar. \
            Press **?** in the top bar to see this guide anytime.\n\n";
        let content = format!(
            "---\ntitle: Welcome\norder: 0\n---\n\n{note}{}",
            render::HELP_BODY
        );
        fs::write(&p, render::normalize_markdown(&content))?;
        println!("create: index.md");
    }
    let hint = dir.display().to_string();
    let hint = if hint == "." {
        String::new()
    } else {
        format!(" {hint}")
    };
    println!("\nDone. Next: caret serve{hint}");
    Ok(())
}

fn build(dir: &Path, out: Option<PathBuf>) -> Result<()> {
    let pages = render::collect_pages(dir)?;
    if pages.is_empty() {
        anyhow::bail!("No .md files found in {}", dir.display());
    }
    let out = out.unwrap_or_else(|| dir.join("index.html"));
    let (theme, settings_json) = config::client_blob(dir);
    fs::write(
        &out,
        render::build_html(&pages, false, &theme, &settings_json),
    )?;
    println!("Built {} pages -> {}", pages.len(), out.display());
    Ok(())
}
