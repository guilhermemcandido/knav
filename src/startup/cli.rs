//! Command-line arguments and startup context resolution.

use crate::*;

/// Command-line arguments, deliberately hand-rolled instead of pulling in
/// a full argument-parsing crate for what's currently a single flag.
pub(crate) struct Cli {
    /// `-c`/`--context <query>`, fuzzy-matched against the kubeconfig's
    /// contexts and connected to directly, bypassing the cluster picker
    /// regardless of `startup.mode`.
    context_query: Option<String>,
}

impl Cli {
    pub(crate) fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut context_query = None;
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-c" | "--context" => {
                    context_query = Some(args.next().with_context(|| format!("{arg} requires a value"))?);
                }
                "-h" | "--help" => {
                    println!(
                        "knav [-c|--context <name>]\n\n  -c, --context <name>  fuzzy-match a kubeconfig context and connect to it directly\n  -h, --help            show this help\n  -v, --version         show the version"
                    );
                    std::process::exit(0);
                }
                "-v" | "--version" => {
                    println!("knav {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                other => anyhow::bail!("unrecognized argument: {other} (try --help)"),
            }
        }
        Ok(Cli { context_query })
    }
}

/// Resolves the context to connect to. `--context` always wins (fuzzy, once);
/// otherwise `startup.mode` picks direct or the cluster picker. `Ok(None)` from
/// the picker means the user cancelled, so knav exits.
pub(crate) fn resolve_context(cli: &Cli, config: &Config) -> Result<Option<String>> {
    if let Some(query) = &cli.context_query {
        let contexts = k8s::list_contexts()?;
        return fuzzy::best_match(query, contexts.iter().map(|c| c.name.as_str()))
            .map(|name| Some(name.to_string()))
            .with_context(|| format!("no kubeconfig context matches '{query}'"));
    }

    match config.startup.mode {
        StartupMode::Direct => Ok(None),
        StartupMode::Menu => {
            let contexts = k8s::list_contexts()?;
            match picker::run(&contexts)? {
                Some(name) => Ok(Some(name)),
                None => std::process::exit(0),
            }
        }
    }
}
