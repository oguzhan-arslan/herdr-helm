//! herdr-helm: the core of Helm, a tiled, project based multi-agent layout for Herdr.
//! See AGENTS.md and docs/ for how it is built and how to change it.

mod assets;
mod cli;
mod commands;
mod connectors;
mod core;
mod paths;
mod util;

fn main() {
    if let Err(e) = cli::run() {
        eprintln!("herdr-helm: {e:#}");
        std::process::exit(1);
    }
}
