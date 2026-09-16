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

Not started. Planning stage — see `TODO.md`.
