//! Command-line arguments: `knav [-c <name>]` launches the TUI; `version`, `update` and `help`
//! are the other things there is to do without a cluster.

use crate::*;

pub(crate) const USAGE: &str = "knav [-c|--context <name>]\n\nLaunches the TUI against your current kubeconfig context, or the one -c names.\n\nCommands:\n  version               show the version (also -v, --version)\n  update [-y|--yes]     update to the latest release (also self-update)\n  help                  show this help (also -h, --help)\n\nOptions:\n  -c, --context <name>  fuzzy-match a kubeconfig context and connect to it directly\n";

pub(crate) enum Cli {
    /// `--context` (fuzzy-matched against the kubeconfig) skips the cluster picker,
    /// regardless of `startup.mode`.
    Launch { context_query: Option<String> },
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
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-c" | "--context" => {
                    context_query = Some(args.next().with_context(|| format!("{arg} requires a value"))?);
                }
                other => anyhow::bail!("unrecognized argument: {other} (try --help)"),
            }
        }
        Ok(Cli::Launch { context_query })
    }
}

/// Resolves the context to connect to. `--context` always wins (fuzzy, once);
/// otherwise `startup.mode` picks direct or the cluster picker. `Ok(None)` from
/// the picker means the user cancelled, so knav exits.
pub(crate) fn resolve_context(context_query: Option<&str>, config: &Config) -> Result<Option<String>> {
    if let Some(query) = context_query {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli> {
        Cli::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn no_arguments_launches_with_no_context() {
        assert!(matches!(parse(&[]).unwrap(), Cli::Launch { context_query: None }));
    }

    #[test]
    fn context_takes_the_next_argument() {
        let Cli::Launch { context_query } = parse(&["-c", "prod"]).unwrap() else { panic!() };
        assert_eq!(context_query.as_deref(), Some("prod"));
        let Cli::Launch { context_query } = parse(&["--context", "prod"]).unwrap() else { panic!() };
        assert_eq!(context_query.as_deref(), Some("prod"));
    }

    #[test]
    fn a_context_with_no_value_is_an_error() {
        assert!(parse(&["-c"]).is_err());
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
