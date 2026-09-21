# knav

A Kubernetes TUI. Not another k9s command-bar clone — the thesis is combining
two ideas that already exist separately, in two different mature tools, but
nowhere together:

1. **Persistent structural navigation** instead of typing `:pods` every time
   (the way `lfk` does it with Miller columns) — the tree/hierarchy is always
   visible, you move through it, you don't recall-and-type resource names.
2. **Proactive "why is this broken" diagnosis** instead of just a status
   table (the way `sofka` and `kwatch` do it) — surfacing cause + suggested
   next step, not just a red/yellow/green cell.

Built in Rust: `ratatui` for rendering, `kube-rs` for the Kubernetes API
client, `tokio` for async (watches/reflectors so views update live instead
of polling).

## Why this, given what already exists

See `TODO.md` for the full prior-art research — several serious, mature
tools already cover pieces of this. This project only makes sense if it
either (a) actually nails the hierarchy-nav + diagnosis combo nobody's
merged yet, or (b) is worth building anyway as a Rust/ratatui/kube-rs
learning project regardless of competition. Worth being honest with
ourselves about which of those it is as it develops.

## Status

Working: a live, k9s-style browser for a cluster, with a Freelens-style set of
columns per resource kind. Read-write (edit with `e`); no delete/scale yet.

## Running

```
cargo run -- [-c|--context <name>]
```

It checks the cluster is reachable and says so cleanly if not. `-c` fuzzy-matches
a kubeconfig context and connects straight to it.

## Keys

Press `?` on any screen for the keys that apply there. The common ones:

| Key | Does |
| --- | --- |
| `:` | Command line (`> `): a kind (`pods`, `rs`, `svc`, ...), `ctx`, `events`, `q` |
| `m` | Menu of every resource kind, including CRDs |
| `n` | Namespace picker; `Enter` picks one and asks which number key (1-9) to keep it on |
| `0`-`9` | Switch namespace: `0` is all, `1`-`9` the ones you reserved (kept per context) |
| `Enter` | Drill into what a row owns (Deployment → ReplicaSets → Pods → containers → logs), or open its spec |
| `/` | Search the list; matches are highlighted |
| `s` then a digit | Sort by that column; the same digit flips the direction. `s`/`Esc`/`q` leaves |
| `←` `→` / `h` `l` | Scroll columns sideways when they don't fit |
| `d` / `e` | Show the spec / edit the manifest in `$EDITOR` |
| `D` | Delete the selected object (asks first) |
| `S` | Scale a Deployment, StatefulSet or ReplicaSet (type the count) |
| `r` | Restart a Deployment, StatefulSet or DaemonSet (asks first) |
| `a` | Shell in a pod's container (in the containers popup: the selected one); needs `kubectl` on the PATH |
| `o` | Cordon or uncordon a node |
| `t` / `u` | Trigger a CronJob now / suspend or resume it |
| `C` | Switch context |
| `c` | Toggle mouse capture (off lets the terminal select text) |
| `:q` | Quit (a stray `q` never does) |

The mouse works too: the wheel moves the selection in every list and popup, a click
selects a row, a double-click opens it, and clicking a tile on the overview selects it.

The bottom bar shows where you are (`Deployment[web]>>ReplicaSets>>...`) and
the selected row's status, with a green or yellow dot for ready counts.

## Config

`$XDG_CONFIG_HOME/knav/config.toml` (default `~/.config/knav/config.toml`). Every
field is optional.

```toml
[startup]
mode = "direct"          # or "menu": always show the context picker first

[logs]
order = "oldest_first"   # or "newest_first"; `o` toggles it in the log view
timestamp_format = "short" # or "full"; `t` toggles it

[keybindings.logs]
toggle_timestamp = "t"
toggle_order = "o"

[tables]
min_column_width = 10    # no column is squeezed below this; wider tables scroll sideways
[tables.min_widths]      # per column, keyed by the lowercase header
name = 24
"up-to-date" = 12
```

Reserved namespaces are saved in `~/.config/knav/namespaces`.

## Layout of the code

- `src/k8s/`: talking to the API and shaping objects into rows (one module per concern)
- `src/describe.rs`: the per-kind columns and their colours
- `src/app/`: the event loop: `state` (what's on screen), `derive` (rows per frame), `draw`, `handlers/` (input per screen)
- `src/ui/`: rendering: `layout` (column widths and scrolling), `tables`, popups, header, breadcrumb
- `src/mode.rs`, `src/sort.rs`, `src/scope.rs`, `src/commands.rs`: screens, sorting, drill-down scope, the command line
