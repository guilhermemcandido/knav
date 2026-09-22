# knav

A fast, mouse-friendly Kubernetes TUI for big clusters. Think k9s with an IDE's
explorer, built in Rust (`ratatui` + `kube-rs`).

![knav in use](docs/demo.gif)

- **Sidebar** of every kind, custom resources included, with live counts.
- **Info panel** (`i`) explains the selected object: status, containers, volumes, events.
- **Relations diagram** (`R`): what an object depends on and what depends on it, zoomable and walkable.
- **Built for scale**: stays fast into the tens of thousands of objects; discovery and counts are batched, not per-kind.
- Mouse and keyboard both work everywhere: click, scroll, double-click.

## Install

```
curl -fsSL https://raw.githubusercontent.com/guilhermemcandido/knav/main/install.sh | sh
```

Or from source (Rust 1.85+):

```
git clone https://github.com/guilhermemcandido/knav && cd knav && cargo install --path .
```

Needs a working kubeconfig; `kubectl` is only used for port-forwards and the in-app shell.

## Use

```
knav              # current kubeconfig context
knav -c prod      # fuzzy-matches a context by name
```

Press `?` on any screen for every key that applies there. The essentials:

| Key | Does |
| --- | --- |
| `:pods` | open any resource the cluster serves (`:svc`, a custom resource, ...) |
| `/` | search the list |
| `i` | info panel for the selected row |
| `R` | what's related to it |
| `Enter` | drill in (Deployment → ReplicaSets → Pods → containers → logs) |
| `y` / `e` / `D` | YAML / edit in `$EDITOR` / delete |
| `l` / `L` | a pod's logs / a whole workload's, merged |
| `0`-`9` | jump to a reserved namespace |
| `b` | the resource sidebar |
| `s` | sort by a column |

## Config

`~/.config/knav/config.toml`, entirely optional. Themes, key bindings and layout are
all editable live from the Settings screen (`,`), which writes changes back for you.

## License

MIT OR Apache-2.0.
