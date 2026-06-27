mod config;
mod render;
mod serve;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "docd",
    version,
    about = "Render a directory of markdown into one interactive HTML page."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scaffold a new docd project (non-destructive; skips existing files)
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
    for (rel, content) in SCAFFOLD {
        let p = dir.join(rel);
        if p.exists() {
            println!("skip (exists): {rel}");
            continue;
        }
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&p, content)?;
        println!("create: {rel}");
    }
    let hint = dir.display().to_string();
    let hint = if hint == "." {
        String::new()
    } else {
        format!(" {hint}")
    };
    println!("\nDone. Next: docd build{hint}");
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

const SCAFFOLD: &[(&str, &str)] = &[
    (
        "index.md",
        r#"---
title: Home
order: 0
---

# My Docs

Welcome to your **docd** workspace — a directory of markdown files rendered
into one interactive page. Edit the `.md` files, re-run `docd build`, refresh.

## Start here

- [Getting Started](guide/getting-started.md)
- [Concepts](guide/concepts.md)
- [Architecture](architecture.md)
"#,
    ),
    (
        "guide/getting-started.md",
        r#"---
title: Getting Started
order: 1
---

# Getting Started

Your first steps in this workspace.

1. Edit any `.md` file.
2. Run `docd build`.
3. Open `index.html`.

Next: [Concepts](concepts.md) · [Home](../index.md)
"#,
    ),
    (
        "guide/concepts.md",
        r#"---
title: Concepts
order: 2
---

# Concepts

The core ideas behind docd.

- **Source of truth**: plain markdown in git.
- **One page**: the whole tree renders into a single SPA.
- **Links**: relative `.md` links become in-page navigation.

Back to [Getting Started](getting-started.md).
"#,
    ),
    (
        "architecture.md",
        r#"---
title: Architecture
order: 3
---

# Architecture

How the pieces fit together.

## Overview

The CLI renders every `.md` file into a single HTML page with client-side
navigation. Relative links between files are rewritten automatically.

Back to [Home](index.md).
"#,
    ),
];
