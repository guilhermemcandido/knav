# knav

A fast, mouse-friendly Kubernetes TUI for big clusters. Think k9s with an IDE's
explorer, built in Rust.

![knav in use](demo/demo.gif)

- **Sidebar** of every kind, custom resources included, with live counts.
- **Info panel** (`i`) explains the selected object: status, containers, events.
- **Relations diagram** (`R`) shows what an object depends on and what depends on it.
- **Extensions** (`E`) add views and dashboards for Flux, Argo CD, cert-manager and more. Read-only.
- **Knows your access**: the header shows your role (admin, read-write, read-only), checked against RBAC.
- **Built for scale**: stays fast with tens of thousands of objects.

## Install

```
curl -fsSL https://raw.githubusercontent.com/guilhermemcandido/knav/main/install.sh | sh
```

Or from source (Rust 1.96+):

```
cargo install --git https://github.com/guilhermemcandido/knav
```

Needs a kubeconfig. `kubectl` is only used for port-forwards and shells.

## Use

```
knav              # current context
knav -c prod      # pick a context by name
knav --read-only  # block every change: delete, edit, scale, shells
knav update       # update to the latest release
```

Press `?` anywhere for the keys on that screen. The essentials:

| Key | Does |
| --- | --- |
| `:pods` | open any resource (`:svc`, a custom resource, ...) |
| `/` | search the list |
| `Enter` | drill in, from a Deployment down to a container's logs |
| `i` / `R` | info panel / related objects |
| `y` / `e` / `D` | YAML / edit / delete |
| `l` / `L` | a pod's logs / a whole workload's |
| `0`-`9` | switch to a reserved namespace |
| `b` | sidebar |

## Config

Everything is set from the Settings screen (`,`) and saved to
`~/.config/knav/config.toml`, which you can also edit by hand:

- **General**: wide or faults-only lists on start, log order and timestamps, mouse, refresh rates
- **Appearance**: theme, box lines, icons (turn them off if your terminal can't draw images)
- **Behaviour**: start in the current context or a picker, read-only mode, port-forwards
- **Keys**: rebind any key
- **Layout**: which sections and tiles Home shows, and in what order
- **Read-only**: always, or only for contexts that match a pattern:

```toml
[read_only]
contexts = ["prod*"]
```

Themes also switch live with `T`. Your own extensions go in `~/.config/knav/extensions/`.

## License

MIT or Apache-2.0.
