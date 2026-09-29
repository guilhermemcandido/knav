//! Command-line arguments: `knav [-c <name>]` launches the TUI; `version`, `update`
//! and `help` work without a cluster.

use anyhow::{Context as _, Result};

use crate::config::{Config, StartupMode};
use crate::k8s;
use crate::startup::picker;
use crate::util::fuzzy;


pub(crate) const USAGE: &str = "knav [-c|--context [name]] [--read-only]\n\nLaunches the TUI against your current kubeconfig context.\n\nCommands:\n  version               show the version (also -v, --version)\n  update [-y|--yes]     update to the latest release (also self-update)\n  help                  show this help (also -h, --help)\n\nOptions:\n  -c, --context <name>  fuzzy-match a kubeconfig context and connect to it directly\n  -c, --context         (no name) pick a context from a list\n      --read-only       block every change: delete, edit, scale, shells\n";

pub(crate) enum Cli {
    /// `--context <name>` (fuzzy-matched) skips the picker whatever `startup.mode` says;
    /// a bare `--context` always shows it.
    /// `--read-only` blocks changes for the whole run, whatever the config says.
    Launch { context_query: Option<String>, pick: bool, read_only: bool },
    Version,
    /// `-y`/`--yes` skips the "update to vX.Y.Z?" confirmation.
    Update { yes: bool },
    Help,
}

impl Cli {
    pub(crate) fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut args = args.peekable();
        match args.peek().map(String::as_str) {
            Some("version" | "-v" | "--version") => return Ok(Cli::Version),
            Some("help" | "-h" | "--help") => return Ok(Cli::Help),
            Some("update") => {
                args.next();
                let mut yes = false;
                for arg in args {
                    match arg.as_str() {
                        "-y" | "--yes" => yes = true,
                        other => anyhow::bail!("unrecognized argument: {other} (try `knav update --yes`)"),
                    }
                }
                return Ok(Cli::Update { yes });
            }
            _ => {}
        }
        let mut context_query = None;
        let mut pick = false;
        let mut read_only = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                // No name after it: show the picker instead of erroring.
                "-c" | "--context" => match args.next() {
                    Some(value) => context_query = Some(value),
                    None => pick = true,
                },
                "--read-only" | "--readonly" => read_only = true,
                other => anyhow::bail!("unrecognized argument: {other} (try --help)"),
            }
        }
        Ok(Cli::Launch { context_query, pick, read_only })
    }
}

/// The context to connect to: a `--context <name>` query first, then the picker for a
/// bare `--context` or `startup.mode = "menu"`, else direct. `Ok(None)` means cancelled.
pub(crate) fn resolve_context(context_query: Option<&str>, pick: bool, config: &Config) -> Result<Option<String>> {
    if let Some(query) = context_query {
        let contexts = k8s::list_contexts()?;
        return fuzzy::best_match(query, contexts.iter().map(|c| c.name.as_str()))
            .map(|name| Some(name.to_string()))
            .with_context(|| format!("no kubeconfig context matches '{query}'"));
    }

    if pick || matches!(config.startup.mode, StartupMode::Menu) {
        let contexts = k8s::list_contexts()?;
        return match picker::run(&contexts)? {
            Some(name) => Ok(Some(name)),
            None => std::process::exit(0),
        };
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli> {
        Cli::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn no_arguments_launches_with_no_context() {
        assert!(matches!(parse(&[]).unwrap(), Cli::Launch { context_query: None, pick: false, read_only: false }));
    }

    #[test]
    fn context_takes_the_next_argument() {
        let Cli::Launch { context_query, pick, .. } = parse(&["-c", "prod"]).unwrap() else { panic!() };
        assert_eq!(context_query.as_deref(), Some("prod"));
        assert!(!pick);
        let Cli::Launch { context_query, pick, .. } = parse(&["--context", "prod"]).unwrap() else { panic!() };
        assert_eq!(context_query.as_deref(), Some("prod"));
        assert!(!pick);
    }

    #[test]
    fn a_bare_context_flag_asks_to_pick_one_instead_of_erroring() {
        let Cli::Launch { context_query, pick, .. } = parse(&["-c"]).unwrap() else { panic!() };
        assert!(context_query.is_none() && pick);
        let Cli::Launch { context_query, pick, .. } = parse(&["--context"]).unwrap() else { panic!() };
        assert!(context_query.is_none() && pick);
    }

    #[test]
    fn read_only_goes_with_a_context() {
        let Cli::Launch { context_query, read_only, .. } = parse(&["-c", "prod", "--read-only"]).unwrap() else { panic!() };
        assert_eq!(context_query.as_deref(), Some("prod"));
        assert!(read_only);
    }

    #[test]
    fn version_help_and_update_are_recognized_anywhere_first() {
        assert!(matches!(parse(&["version"]).unwrap(), Cli::Version));
        assert!(matches!(parse(&["-v"]).unwrap(), Cli::Version));
        assert!(matches!(parse(&["--version"]).unwrap(), Cli::Version));
        assert!(matches!(parse(&["help"]).unwrap(), Cli::Help));
        assert!(matches!(parse(&["-h"]).unwrap(), Cli::Help));
        assert!(matches!(parse(&["update"]).unwrap(), Cli::Update { yes: false }));
        assert!(matches!(parse(&["update", "--yes"]).unwrap(), Cli::Update { yes: true }));
        assert!(matches!(parse(&["update", "-y"]).unwrap(), Cli::Update { yes: true }));
    }

    #[test]
    fn an_unknown_flag_is_an_error() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["update", "--nope"]).is_err());
    }
}
