# knav — TODO

## Prior art (checked 2026-09-16 — re-verify star counts/activity before assuming these are current)

Direct competitors — k8s browsing/management TUIs:

- **k9s** (derailed/k9s) — https://github.com/derailed/k9s — the baseline.
  Command-bar navigation (`:pods`, `:deploy`, ...). Huge plugin ecosystem
  and mindshare. Has hidden visual views worth studying: `:pulse` (health
  dashboard, sparklines, tracks 16 resource kinds with real status fields —
  deliberately excludes ConfigMaps/Secrets since those have no health
  state) and `:xray` (dependency tree view for a single resource).
- **sofka** (nklmilojevic/sofka) — https://github.com/nklmilojevic/sofka —
  1,187★. Rust, ratatui, kube-rs — same stack we're planning. Same
  command-bar nav as k9s (didn't break from that model). What it adds:
  Flux CD / Argo CD native actions (suspend/resume/reconcile), an
  "evidence-based incident view" (why something's broken, not just that it
  is), bulk operations, non-blocking background port-forwarding, a
  read-only safety mode + delete guardrails, multiple skins.
- **lfk** (janosmiko/lfk) — https://github.com/janosmiko/lfk — 893★, Go.
  **Already builds the persistent-navigation idea we wanted.** Three-column
  Miller-columns layout (file-manager style): Clusters → Resource Types
  (grouped: Workloads/Networking/Config/Storage/ArgoCD/Helm/...) →
  Resources → Owned Resources (via ownerReferences) → Containers. Owner
  hierarchy auto-resolves (Deployment→ReplicaSet→Pods flattened, CronJob→
  Jobs, Service→matching Pods via selector). Namespace is an orthogonal
  filter (`\` to open selector), not part of the tree. Huge action set:
  logs/exec/attach/debug/scale/restart/delete/describe/edit/events/
  port-forward/vuln-scan/PVC-resize/Pod-resize, RBAC-aware filtering,
  cascade-delete modes. CRD auto-discovery grouped by API group. Mature:
  1,152 commits, e2e tests, goreleaser CI, SonarCloud/OpenSSF Scorecard,
  460+ themes. Real production users.
- **ku** (bjarneo/ku) — https://github.com/bjarneo/ku — 553★. Fast,
  keyboard-driven. Browse any resource, edit objects, follow logs, shell
  into pods.
- **kdash** (kdash-rs/kdash) — https://github.com/kdash-rs/kdash — 2,534★,
  Rust. "Simple and fast dashboard for Kubernetes." Highest-starred Rust
  k8s TUI found. Haven't dug into its navigation model yet — check before
  building, might already cover ground we think is open.
- **kubetui** (sarub0b0/kubetui) — https://github.com/sarub0b0/kubetui —
  393★. "Intuitive TUI for real-time monitoring and exploration."
- **buoy** (everettraven/buoy) — https://github.com/everettraven/buoy —
  81★. "A declarative Kubernetes dashboard in your terminal."

Adjacent / inspiration (not direct competitors, but relevant features):

- **kftray** (hcavarsan/kftray) — https://github.com/hcavarsan/kftray —
  1,562★. kubectl port-forward manager + reverse tunnel, GUI *and* TUI.
  Worth studying for the port-forward-manager UX specifically.
- **kwatch** (abahmed/kwatch) — https://github.com/abahmed/kwatch — 1,018★.
  NOT a TUI — an in-cluster incident *monitor* that pushes alerts
  (Slack/Discord) explaining what broke, why, and next steps. This is
  where the "diagnosis" half of our thesis is proven out, just not in a
  terminal-viewer context. Also: the name `kwatch` is taken multiple times
  over (two more, smaller TUIs also use it: tituscarl/kwatch, panos--/kwatch)
  — do not use this name.
- **tituscarl/kwatch** — https://github.com/tituscarl/kwatch — 8★. Small
  TUI for monitoring k8s services, pods, deployments.
- **panos--/kwatch** — https://github.com/panos--/kwatch — 3★. Another
  small "Kubernetes terminal UI."
- **Freelens** (open-source Lens fork) — the original inspiration for this
  whole idea. Full desktop GUI, not a TUI — multi-cluster tree sidebar,
  resource forms, metrics graphs. What we're chasing is "the useful parts
  of this, in a terminal."
- **kview** (michaeljsaenz/kview) — https://github.com/michaeljsaenz/kview
  — desktop GUI app (Go), not a TUI. Actively maintained. Noted here mainly
  because it's the reason we ruled out the name `kview` for this project.

## Decisions made

- [x] Language/stack: Rust + ratatui + kube-rs + tokio (see README)
- [x] Name: `knav` (checked clean on crates.io and GitHub, 2026-09-16)
- [x] Location: `~/Desktop/Work/knav`
- [x] MVP scope target: read-only viewer first (no exec/edit/delete yet)
- [x] License check on all prior-art repos (2026-09-16): k9s, sofka, lfk —
      Apache-2.0. ku, kdash, kubetui, abahmed/kwatch — MIT. buoy,
      tituscarl/kwatch, panos--/kwatch — Apache-2.0. **kftray is GPLv3
      (copyleft)** — fine to study for UX ideas, do not copy its code into
      knav unless the whole project goes GPLv3 too. knav itself is
      dual-licensed MIT OR Apache-2.0 (standard Rust ecosystem convention,
      matches ratatui/kube-rs themselves) — see `LICENSE-MIT`/`LICENSE-APACHE`.
- [x] Scaffolded: `cargo init`, deps added (`ratatui`, `crossterm`, `kube`
      with `runtime`+`derive` features, `k8s-openapi` with `latest`,
      `tokio` full, `anyhow`). Builds clean.
- [x] v0.1 built: `src/k8s.rs` (connect + list all pods via kube-rs, maps
      to a plain `PodRow`), `src/ui.rs` (ratatui `Table`, phase-colored:
      green/Running, yellow/Pending, red/Failed), `src/main.rs` (event
      loop: `q`/`Esc` quit, `r` refetches). No watch/reflector yet — `r` is
      a one-shot re-fetch, not live streaming.
- [x] Verified against a real cluster (throwaway k3d cluster via colima,
      2026-09-16): the k8s/data layer output matches `kubectl get pods -A`
      exactly across namespaces/phases/restart counts. Could NOT visually
      confirm the actual TUI rendering from the agent sandbox (no real tty
      available there) — ui.rs compiled clean and is straightforward
      widget composition, but a human should eyeball it in a real
      terminal to confirm layout/colors look right. The test cluster is
      still running (`k3d cluster delete knav-test` to tear down, colima
      stop to stop the VM) specifically so this can be done.

## Done since last update (2026-09-16)

- [x] **Live view implemented.** `k8s::watch_pods` uses a kube-rs
      `watcher` + `reflector::store()` running in a background
      `tokio::spawn` task, with `.default_backoff()` for reconnects. No
      more manual `r`-to-refresh — `main.rs` just reads `store.state()`
      every render tick. Proven against the real cluster: a pod created
      mid-run showed up with zero manual refetch calls (see git history /
      this session for the smoke-test transcript, not saved to disk).
- [x] Populated the test cluster with more variety: three new namespaces
      (`shop`, `payments`, `staging`) with pods covering every phase —
      Running, Pending (`shop/oversized`, impossible resource request),
      Failed (`payments/migration-job`, `restartPolicy: Never` exiting 1),
      and a crash-looping one (`staging/flaky`, restarts climbing).
- [ ] **Found while doing that, worth remembering:** `staging/flaky` is
      crash-looping (3 restarts and climbing) but its `.status.phase` is
      still `Running` — Kubernetes' pod phase doesn't have a distinct
      CrashLoopBackOff state, that's a container-status *reason*, not the
      pod phase. Our current color logic only reads `phase`, so a
      crash-looping pod currently renders **green** in knav right now,
      same as healthy. This is a real gap, and it's a concrete example of
      exactly the kind of thing the "diagnosis" half of the thesis (see
      README) is supposed to fix — a naive status field lies here, same
      as it would in any k9s-style tool. Worth fixing the color logic to
      also check `container_statuses[].state.waiting.reason` before this
      goes much further.

## Done since last update (2026-09-16, later same day)

- [x] **Scrolling/selection implemented.** `TableState` drives the pod
      table now (`j`/`k`/arrows move a highlighted row, clamped — no
      wraparound at top/bottom). Selection is clamped every tick against
      the live row count too, so it can't point past the end if pods
      disappear out from under it.
- [x] **Spec view implemented.** `Enter` on a selected pod opens a
      centered popup (85% of the screen) showing that pod's full manifest
      as YAML (`k8s::pod_yaml`, via `serde_yaml` — note: this crate is
      tagged `+deprecated` upstream, still functional, but a maintained
      replacement is worth a look eventually), with `managedFields`
      stripped since it's unreadable noise. Lightly colorized line-by-line
      (cyan keys, magenta list dashes) — not a real YAML parser, just
      enough to make it visually scannable. `j`/`k` scrolls the popup,
      `Esc`/`q` closes it back to the list (only quits the whole app from
      List mode).
- [x] Unit-tested the selection boundary logic directly (`cargo test`,
      4 tests: bottom/top clamping, empty list, single-item list) since
      the interactive loop itself can't be exercised without a real tty.
      Verified `pod_yaml` output against the real `flaky` pod in the test
      cluster — confirmed clean (no managedFields) and, notably, the raw
      YAML already shows `restartCount: 6` and
      `state.terminated.reason: Error` right there in
      `status.containerStatuses` — exactly the data a future "diagnosis"
      feature would read to catch what the phase-only color logic still
      misses (see the `staging/flaky` note above, still unfixed).

## Done since last update (2026-09-16, later still)

- [x] Background dims (muted to dark gray — border, header, status
      colors, selection highlight, cursor symbol) while the spec popup is
      open, so it reads as a proper modal instead of the popup just
      floating over an unchanged table. True blur isn't a thing terminals
      can do; this is the standard substitute.

## Done since last update (2026-09-16, later still)

- [x] **Collapsible spec view, Freelens-style.** Replaced the flat
      colorized-text popup with a real tree (`tui-tree-widget`, MIT,
      compatible with our ratatui 0.30.x). The pod's manifest is parsed
      into a generic `serde_yaml::Value` (`k8s::pod_value`) and walked
      into a `Vec<TreeItem<String>>`, one identifier per field path (e.g.
      `root/spec/containers/[0]/image`) so it stays unique even though
      sibling branches reuse field names like `name`. Verified against
      the real `flaky` pod: 153 nodes (105 leaves), correct nesting, no
      panics.
  - Keyboard: `j`/`k`/arrows move the selection, `h`/`l`/left-right
    collapse/expand, `Enter`/`Space` toggles the selected node.
  - **Mouse**: clicking any row toggles it directly — enabled
    `EnableMouseCapture` in `main.rs` (disabled again on exit), added
    `ui::click_tree` which uses `TreeState::rendered_at` for hit-testing
    (no manual row math) then explicitly calls `select` + `toggle`, since
    the library's own `click_at` only toggles on a *second* click of an
    already-selected row — we want every click to toggle immediately, so
    we bypassed `click_at` and drive `select`/`toggle` directly. Mouse
    wheel scrolls too.
  - Top-level keys (`apiVersion`, `kind`, `metadata`, `spec`, `status`)
    open by default so the popup isn't a single collapsed line the first
    time you see it.

## Done since last update (2026-09-16, later still #2)

- [x] **Answered: field order (spec before status) is not something we
      changed.** Checked live: `kubectl get pod -o yaml` shows the exact
      same `apiVersion, kind, metadata, spec, status` order. That's
      standard everywhere (kubectl, k9s's `y`) — spec is "what you asked
      for," status is "what happened," always reported after. Not a bug.
- [x] **Colored per-container status dots in the main pod list**, Freelens
      style — new CONTAINERS column, one `●` per container colored by
      its *container*-level state (green=Running, yellow=Waiting, red=
      Terminated, gray=Unknown), from `k8s::containers_for` reading
      `status.containerStatuses[].state` directly. **This is the actual
      fix for the `staging/flaky` gap flagged earlier** — phase-only
      coloring said green/healthy; a crash-looping container's `state` is
      `Waiting { reason: CrashLoopBackOff }`, which now shows as a
      distinct-colored dot right in the list, no more phase-only lie.
- [x] **Hover, mouse-driven**: moving the mouse over a row (via
      `MouseEventKind::Moved`, needs `EnableMouseCapture`) shows that
      pod's per-container name+state as the table's title — a terminal
      has no real tooltip primitive, so this is the practical substitute
      for "hoverable." `ui::row_at` maps an absolute terminal position to
      a row index (accounts for border/header offset and scroll); it's
      row-granularity (whole row lights up the caption), not per-dot —
      per-dot hit-testing inside a multi-symbol cell would need more
      fragile column math than was worth it here.
- [x] **New navigation flow, closer to k9s/Freelens conventions:**
      `d` (was `Enter`) opens the pod spec tree. `Enter` now drills into
      a **Containers view** (name, colored state, restarts — its own
      small table). `Enter` again on a container opens **live-tailing
      logs** (`k8s::stream_logs`, a background task using kube-rs's
      `Api::log_stream` piped through `futures::AsyncBufReadExt::lines()`
      — turns out `log_stream` already returns something bufread-able
      directly, no extra stream-adapter crate needed). `Esc`/`q` backs
      out one level at a time (Logs → Containers → List), never straight
      to quit except from List itself. Verified against a real chatty
      pod: captured 34 lines (existing history + live follow) in a
      4-second window, matching `kubectl logs -f` semantics.
- [x] Confirmed clean: `cargo build`, `cargo test` (4/4), `cargo clippy
      --all-targets` — zero warnings across all of the above.

**Honest caveat, same as always:** none of the *visual* parts (dot
colors/positions, hover caption placement, whether `row_at`'s math is
off by a row somewhere) have been eyeballed in a real terminal — this
sandbox still has no real tty. The data/logic side is verified
end-to-end against the live cluster; the pixel-level layout isn't.
**Worth specifically checking by hand:** does hovering actually show the
caption without a stray offset, does clicking a container dot... (there's
no click-on-dot behavior — only row hover, drilling in is via `Enter`,
not clicking the dot itself. If you wanted clicking a *specific* dot to
jump straight to that container's logs, that's not built — say so if you
want it.)

## Done since last update (2026-09-16, later still #3)

- [x] **Fixed the hover bug you hit.** Root cause: mouse *motion* events
      (hover without a button held) aren't reliably delivered by every
      terminal — macOS's built-in Terminal.app is a known offender, even
      with `EnableMouseCapture` on. Fix: the container-state caption now
      falls back to the keyboard-*selected* row whenever there's no (or
      unreliable) mouse hover data, so it always works regardless of
      terminal mouse support. Also moved it out of the table's title
      (where it was overwriting the `d`/`enter`/`q` keybinding hints
      almost all the time) into its own persistent status line below the
      table, now colorized (dot + name + state, matching the row's dot
      colors) instead of plain text.
- [x] **Logs, several improvements:**
  - **Auto-follow.** This was the biggest real gap — logs weren't
    tailing, they just sat wherever you'd last scrolled. Now defaults to
    "following" (always shows the tail, computed fresh each frame from
    the actual visible height rather than trusting `Paragraph`'s scroll
    clamping) — pressing `j`/`k`/arrows/mouse-wheel pauses it, `G`
    resumes. Title shows which mode you're in.
  - **Timestamps.** `LogParams.timestamps = true` — server-side
    timestamps (more trustworthy than "when knav happened to read this
    line"), verified live against the real cluster.
  - **Fixed a real navigation bug against my own docs:** I'd written
    "Esc backs out one level at a time (Logs → Containers → List)" but
    the code actually jumped straight to List. Fixed for real this time
    — `Mode::Logs` now carries a `back: Box<Mode>` snapshot of the
    Containers view it came from, restored on Esc via
    `mem::replace`.
- [x] Build/tests(4/4)/clippy all clean throughout (one clippy nit fixed:
      `TableState` is `Copy`, was needlessly `.clone()`d).

**Same caveat as always:** the pixel-level rendering (does the status
line actually line up, does the tail-follow math produce the right
number of visible lines) isn't eyeballed from this sandbox — worth
trying yourself.

## Done since last update (2026-09-16, later still #4)

- [x] **Corrected myself on Ghostty.** Looked it up rather than agreeing
      with the "probably has the same issue" guess: Ghostty is a modern,
      spec-compliant terminal, specifically called out in search results
      as a *correct* reference implementation of mouse motion tracking
      (mode 1003) — unlike some buggy terminals. Confirmed `crossterm`'s
      `EnableMouseCapture` does request mode 1003 too. So the earlier
      "terminal doesn't report motion" explanation was a plausible-sounding
      guess that's likely wrong for Ghostty specifically — the real cause
      is still unknown.
- [x] **Added temporary debug instrumentation** to actually find the real
      cause instead of guessing again: a bottom line (marked `DEBUG mouse:`
      in magenta) showing the raw last mouse event — kind, column, row,
      frame height, and what `row_at` computed from it. **This needs to be
      read by a human in a real terminal and then removed** — search for
      `TEMP DEBUG` in `main.rs`/`ui.rs` to find both spots.
- [x] **Redesigned hover to be in-place, per explicit feedback** ("I want
      to see the container... in place, not just underneath"): dropped
      the separate status-line-below-the-table approach entirely. Now the
      hovered/selected row's own CONTAINERS cell expands from compact dots
      to full `name: state` text (still colored the same as the dot) —
      right in the row you're already looking at, not elsewhere on screen.
      Widened that column (22%→38%) to give the expanded text more room;
      it still just clips gracefully if a pod has enough containers to
      overflow it. `hovered.or(selected)` fallback logic moved from
      `main.rs` into `ui::draw` itself (cleaner: main.rs just passes the
      raw hover state, ui.rs owns the display policy).
- [x] Build/tests(4/4)/clippy all clean.

**Needs a human with a real terminal for this one specifically:** run
`cargo run`, move the mouse over the pod table, and read the magenta
debug line at the bottom. If it never changes at all, Ghostty (or
something in between) isn't delivering mouse events to the app the way
expected — if it changes but `row_at` computes the wrong index, it's a
math bug in `ui::row_at`. Report back what it shows and the real fix
should be quick either way.

## Done since last update (2026-09-16, later still #5)

- [x] **Debug instrumentation confirmed the mechanism works and was
      removed as promised.** The Ghostty hover issue is resolved.
- [x] **Redesigned hover again, per explicit feedback** ("should only show
      up when my mouse is on it like a popup in front, otherwise show in
      the bottom like you had before"):
  - The bottom status line is back, showing the **keyboard-selected**
    row's container breakdown (reliable regardless of mouse support) —
    this is the "otherwise" default state now, same as before the
    in-place-cell-expansion detour.
  - **New: a real floating popup**, positioned right next to the cursor
    (`ui::popup_near`, clamped so it never renders past the terminal's
    right/bottom edge), appearing *only* while the mouse is actively
    hovering a row — separate from and in addition to the bottom line.
    This replaces the in-place-cell-expansion approach from the previous
    round, which didn't match what was actually wanted.
- [x] **Checked what k9s/Freelens actually show by default before adding
      anything**, rather than assuming: fetched k9s's own
      `defaultPodHeader` from source, and Freelens' recent changelog.
      **Correction on "replica set":** neither tool shows an owner/
      ReplicaSet column in the pod *list* by default — that's detail-view
      info only (which we already have, via `d`'s
      `metadata.ownerReferences`). Freelens' own maintainers actually
      *removed* NODE/QoS/IP from their default list columns recently to
      reduce clutter. What's genuinely standard in both: **READY**,
      **NODE**, **AGE** — added all three.
  - `PodRow` gained `ready` ("2/3" style, from
    `container_statuses[].ready`), `node` (`spec.nodeName`), `age`
    (humanized "5m"/"3h"/"2d", single dominant unit like kubectl/k9s).
  - Needed a duration type: k8s-openapi 0.28 turned out to use `jiff`
    internally, not `chrono` — added `chrono` first, hit a type-mismatch
    compile error, removed it and used the `jiff::Timestamp` that was
    already there instead of pulling in a second date/time crate for the
    same job.
  - Verified against the real cluster: matches `kubectl get pods -o wide`
    exactly, including the edge case (`shop/oversized`, still `Pending`
    and never scheduled) correctly showing `0/0` ready and `-` for node
    instead of garbage or a crash.
- [x] Table columns widened from `Percentage` to a `Length`/`Fill` mix
      (narrow fixed-width for READY/RESTARTS/AGE, proportional `Fill` for
      the text-heavy columns) — the percentage-only approach didn't scale
      to 8 columns.
- [x] Build/tests(4/4)/clippy all clean.

**Not added, on purpose:** CPU/MEM columns (both tools have them, but
they need `metrics-server` integration — a separate, bigger effort, not
just a data-mapping addition like the others). QoS class and Pod IP,
since Freelens itself now hides those by default.

## Done since last update (2026-09-16, later still #6)

- [x] **Hover precision fixed** — was triggering on the whole row,
      should've been just the container dots. Real fix, not another
      guess: `ui::row_at` now reuses `Table`'s own column `Constraint`s
      through an actual `Layout::horizontal(...).spacing(1).split(...)`
      call (factored into `pod_table_widths()`, shared with the real
      table so they can't drift apart) to get the exact x-range of the
      CONTAINERS column, and only returns a hit inside that range.
      Manual pixel-math guessing would've been fragile; this reuses
      ratatui's actual layout solver instead.
- [x] **Central "switch resource" menu, Freelens-style**: press `m` from
      the pod/deployment list to open it. Rounded-corner tiles
      (`BorderType::Rounded`), grouped into sections (`Workloads` today —
      more sections/kinds are just adding another `MenuSection`/tile, per
      the data model already built for it, not a restructure).
      `h`/`l`/←→ moves between tiles, `Enter` switches the active
      resource kind and returns to the list (selection resets to the
      top), `Esc`/`q` cancels without changing anything. Reopening the
      menu highlights whichever kind is currently active.
- [x] **Deployments added as the second resource kind** — proves the
      menu/multi-kind pattern actually generalizes, not just Pods with
      extra UI around it:
  - `k8s::DeploymentRow`/`row_for_deployment`/`watch_deployments`/
    `snapshot_deployments`, mirroring the Pod versions. Columns:
    NAMESPACE, NAME, READY (ready/desired replicas), UP-TO-DATE,
    AVAILABLE, AGE — matches `kubectl get deployments` exactly, verified
    against the real cluster's 4 existing Deployments (coredns,
    local-path-provisioner, metrics-server, traefik).
  - `d` (spec view) works for Deployments too — `manifest_value` and
    `build_manifest_tree` (renamed from `build_pod_tree`, since it never
    actually was pod-specific — it just walks a generic YAML value tree)
    were already resource-kind-agnostic, so this needed zero new tree
    logic, just wiring.
  - `Enter` (containers/logs drill-down) intentionally does nothing for
    Deployments — that's a Pod-level runtime concept, doesn't map
    directly. Not a bug, a scope decision for this round.
  - Deployments have no per-row "containers" concept, so the hover
    popup/status-line/CONTAINERS-column logic simply doesn't apply to
    that view — `ui::Rows` enum branches per resource kind so Pod-only
    UI pieces aren't drawn at all when viewing Deployments, rather than
    rendered empty/broken.
- [x] Build/tests(4/4)/clippy all clean. Verified against the live
      cluster: Deployment rows match `kubectl get deployments -A`
      exactly, and `manifest_value`'s managedFields-stripping confirmed
      working generically (not just for Pods).

**Not done, on purpose, for scope:** only one menu section (`Workloads`)
and one additional kind (Deployments) — "and so on" could mean
ConfigMaps, Secrets, Services, StatefulSets, DaemonSets, Ingresses, Jobs,
CronJobs, Nodes, Namespaces... a lot of ground, each needing its own
Row/watch/table (and for some, a meaningfully different Enter/drill-down
story, e.g. a StatefulSet's Enter might reasonably drill into pods like a
Deployment's could, but a ConfigMap's Enter doesn't map to anything
pod-like at all). Worth deciding the next batch explicitly rather than
guessing which ones matter most.

## Done since last update (2026-09-16, later still #7)

- [x] **Log lines colorized**: timestamp (added earlier via
      `timestamps: true`) is now split off and de-emphasized in dark
      gray, and the message itself is heuristically colored by scanning
      for keywords — red for error/fatal/panic/fail, yellow for warn,
      gray otherwise. Explicitly **not** a real stdout/stderr distinction
      — Kubernetes' log API merges both streams and doesn't preserve
      which one a line came from, so there's no real signal to key off;
      this is the same keyword-substring approach most terminal log
      viewers fall back to for the same reason.
- [x] Added `ui::colorize_log_line` + `looks_like_timestamp` (cheap shape
      check, not a full RFC3339 parse) and unit-tested both directly
      (`cargo test`, 8/8 now — 4 new): timestamped error line splits
      correctly and colors red, warning colors yellow, plain colors gray,
      and a line with no timestamp (like our own
      `[failed to start log stream: ...]` messages) gets no timestamp
      span but still colors correctly off its own content.
- [x] Build/tests/clippy all clean.

## Done since last update (2026-09-16, later still #8)

- [x] Timestamp now shown as `[timestamp]` in **cyan** (matching the
      spec-tree view's metadata/key color, for consistency across the
      app) instead of dark gray — freeing up gray to mean what it should:
      normal-severity message text, not "also the timestamp." Explained
      the coloring mechanism directly since it was asked: substring
      match on the lowercased message — error/fatal/panic/fail → red,
      warn → yellow, else gray. Naive, not a real log-level parser (a
      line saying "no errors occurred" would still show red).
- [x] Updated the unit test asserting the exact span content/color for
      the bracketed timestamp; all 8 tests still pass, clippy clean.

## Done since last update (2026-09-16, later still #9)

- [x] **Config file infrastructure, staged as discussed** (not the
      fully-generalized "every keybinding remappable" version yet — see
      the "why staged" note below): new `src/config.rs`, TOML, read from
      `$XDG_CONFIG_HOME/knav/config.toml` (falling back to
      `~/.config/knav/config.toml`) — **not** what the `dirs` crate would
      pick on macOS (`~/Library/Application Support`), deliberately, to
      match this machine's actual nvim/tmux/herdr convention instead of
      "technically platform-correct." A real starter file now exists at
      `~/.config/knav/config.toml`, documented inline.
  - No file, or any field left out of one that exists → default. Never
    required. `#[serde(default)]` throughout for partial overrides.
  - Malformed TOML doesn't crash the app — prints a warning (before the
    TUI takes the screen, since after that a plain stderr line would
    never be seen) and falls back to defaults.
  - Verified all three paths directly against a scratch binary with a
    fake `$HOME`: no file → short/`t` defaults, valid override
    (`timestamp_format = "full"`, `toggle_timestamp = "x"`) → read
    correctly, malformed file → exact warning text shown, clean fallback.
- [x] **`[logs] timestamp_format`** (`"short"`/`"full"`, default short)
      and **`t`** in the logs view to toggle between them live —
      `short_timestamp` converts `2026-09-16T18:36:38.477289255Z` to
      `18:36:38.477` (drops the date, truncates nanoseconds to
      milliseconds — as much precision as a human reading logs by eye
      can actually use). Both the default format and the toggle key
      itself are configurable, proving the pattern end-to-end.
  - Unit-tested (`cargo test`, 9/9 now — 1 new): short format truncates
    correctly.
- [x] Build/tests/clippy all clean (one clippy nit: `TimestampFormat`'s
      manual `Default` impl replaced with `#[derive(Default)]` +
      `#[default]` on the variant).

**Why staged, not "every keybinding configurable" right now:** every
other keybinding in the app is still a hardcoded `KeyCode::Char('x')`
match arm. Generalizing *all* of them means a real keybinding-resolution
layer — parsing config strings like `"ctrl+t"`, mapping action names
onto what are currently inline match arms — which is a genuine rewrite
of input handling, not a small addition. This proves the config-file
shape (location, TOML, optional/partial, graceful failure) end-to-end on
one real setting first, so if the shape's wrong it's cheap to fix before
building the bigger thing on top of it.

## Done since last update (2026-09-16, later still #10)

- [x] **Home/Overview dashboard**, matching what k9s's `:pulse` and
      Freelens's Overview actually show — checked both directly against
      real source before building anything, not assumption:
  - k9s `:pulse` = per-kind Total/Faults scorecards, refreshed every 10s.
  - Freelens Overview = `[metrics pie charts]` + `ClusterIssues`, and the
    pie charts are **entirely metrics-server-dependent** (confirmed from
    `cluster-pie-charts.tsx` — explicit "No metrics"/"No Nodes Available"
    fallback states). `ClusterIssues` (`cluster-issues.tsx`) is the part
    shown even without metrics: Node warning conditions + Warning-type
    Events, not a container-status breakdown as originally guessed.
  - Built to match: a row of rounded-corner count tiles (Pods,
    Deployments, Nodes, Namespaces) + a Cluster Issues panel below (Node
    conditions where `Ready != True` or any pressure/unavailable
    condition is `True`, plus Warning events), empty state "✓ No issues
    found." No CPU/Mem/pie charts — still no metrics-server integration,
    consistent with the earlier decision.
  - New as `ResourceKind::Overview`, in its own "Cluster" menu section
    (separate from "Workloads": Pods/Deployments) — reused the existing
    menu/switching infrastructure entirely, no new architecture needed.
    **Now the default screen on launch**, matching "home screen."
  - Needed two new resource watches: `watch_nodes`/`watch_events`
    (same reflector pattern as Pods/Deployments — now at 4 near-identical
    watch functions, genuinely worth genericizing next time this pattern
    gets touched, per the "3 similar cases" threshold).
  - Namespace count is an **approximation** — union of namespaces seen
    across pods/deployments, not a dedicated Namespace watch. A
    namespace with zero pods/deployments in it wouldn't be counted.
    Noted rather than silently wrong; a real Namespace watch is a small
    follow-up if this matters.
- [x] **Found and fixed a real bug while verifying against the live
      cluster**: Kubernetes' newer event format tracks repeat
      occurrences via `series.lastObservedTime`, not `eventTime`/
      `lastTimestamp` (those only capture the *first* occurrence). Our
      first pass read `eventTime` first, so a warning that had been
      recurring for 4 hours showed as "4h" old instead of "how recently
      did this last happen" (23m, matching `kubectl get events`'s LAST
      SEEN column exactly after the fix) — backwards for a panel whose
      whole point is "what needs attention right now."
  - Verified against the real cluster's actual warning events (a real
    `FailedScheduling` on `oversized`, a real `BackOff` on `flaky`) and
    counts (17 pods, 4 deployments, 1 node, 5 namespaces) — all correct,
    including after the timestamp fix.
- [x] Build/tests(9/9)/clippy all clean.

## Done since last update (2026-09-17) — Overview redesigned, real metrics-server integration

Full redesign per explicit feedback ("shouldn't be like [count tiles], should
be a top rectangle with CPU/RAM, then smaller squares below divided by
sections, fixed size, 5-6 per line, wrap to next line, k8s symbols with name
underneath"). Confirmed scope beforehand rather than guessing on the three
open questions (build real metrics now vs. placeholder; short-name
abbreviations vs. icon glyphs; just-implemented kinds vs. full catalog) —
answers were: build real metrics now, icon/emoji glyphs, full catalog.

- [x] **Real metrics-server integration** (`src/metrics.rs`, new module):
  - Kubernetes quantity parsers for CPU (`"250m"`, `"2"`, and the nanocore
    form metrics-server actually reports for live usage, `"123456789n"`)
    and memory (binary Ki/Mi/Gi/Ti and decimal k/M/G/T suffixes, or plain
    bytes) — unit-tested directly (6 tests), since these are pure
    functions I can verify without a cluster.
  - `watch_node_metrics`: polls `metrics.k8s.io/v1beta1/nodes` via
    `kube::api::DynamicObject` (this API isn't in k8s-openapi's generated
    types — it's not a core/stable API group) on a 15s timer, published
    via `tokio::sync::watch` rather than the reflector pattern — **the
    metrics API has no watch support at all**, it's polling-only,
    computed periodically from kubelet cAdvisor stats server-side. `None`
    (not zero) when metrics-server isn't reachable, so the UI can show
    "metrics unavailable" honestly instead of a misleading 0%.
  - Verified against the real cluster: 73m CPU / ~903MiB memory usage,
    matching `kubectl top nodes`' 79m/902Mi (small variance is just
    normal live-metric timing, not a bug) — confirmed via a scratch
    binary, not assumed.
- [x] **Full Freelens-style resource catalog**, ~24 kinds across 6
  sections (Cluster, Workloads, Config, Network, Storage, Access
  Control). Only Pods/Deployments/Nodes have real interactive
  views/reflectors; the other ~20 get **real counts** (not fake
  placeholders) via one new generic function, `k8s::watch_count<K>` —
  polls `Api::<K>::all().list()` on a 15s timer for any resource kind at
  all, one line of code per kind to add (`ReplicaSet`, `StatefulSet`,
  `ConfigMap`, `ClusterRole`, etc.) instead of hand-writing ~20 near-
  identical watchers. This is the genericization of the reflector-pattern
  duplication flagged as worth doing a few rounds back, done — but for
  the *count-polling* pattern specifically, not the full reflector one
  (those still don't have a 3rd real usage yet).
  - Verified 6 representative kinds (spanning namespaced/cluster-scoped,
    different API groups) against the real cluster — namespaces=7,
    replicasets=4, statefulsets=0, daemonsets=1, configmaps=14,
    clusterroles=76 — all matched `kubectl get <kind> -A` exactly.
- [x] **Layout matching the actual request**: a "Cluster Resources" panel
  on top with 3 `ratatui::widgets::Gauge` bars (CPU/Memory/Pods, real
  usage-vs-allocatable ratios, color-coded green/yellow/red by how full),
  falling back to a plain "metrics unavailable" message when
  `metrics_available` is false. Below it, a scrollable area of
  **fixed-size** rounded tiles (icon + live count + kind name), flowing
  left-to-right and wrapping to the next line based on actual terminal
  width (`cols = area.width / TILE_WIDTH`, not a hardcoded "5 per row") —
  grouped under section headers, ending with the Cluster Issues list from
  the previous round (kept, since dropping it wasn't asked for — flagged
  this choice explicitly at the time rather than silently deciding).
  - Scrolling is row-*index*-based, not pixel/line-based — each virtual
    row (a section header, a row of tiles, an issue line) has a
    different height, so "scroll down 1" skips one whole virtual row
    rather than doing partial-row clipping math, which would have been
    much more failure-prone to get right without being able to visually
    verify it here.
- [x] **Icons are my own choice, not an official standard** — flagged
  this before building, per your answer to the clarifying question.
  There's no official terminal-renderable Kubernetes icon set; I picked
  one recognizable emoji per kind (📦 Pods, 🚀 Deployments, 🖥 Nodes, etc.
  — full mapping in `ui::icon_for`). Easy to swap if any don't land right
  for you.
- [x] Build/tests(15/15, 6 new for the quantity parsers)/clippy all clean.

**Committed as its own unit** (per the earlier discussion about actually
using git properly going forward) rather than folded into the batched
initial commit.

## Done since last update (2026-09-17, later) — Overview tiles are selectable

Overview only supported scrolling, not selecting a specific tile — added
real 2D grid selection, keyboard *and* mouse, per explicit request.

- [x] **Keyboard**: arrows (and h/j/k/l) move the selection across the
  actual flow-wrapped grid, including crossing section boundaries (Right
  at a section's last tile moves into the next section's first tile; Up
  from a section's first row moves into the previous section's last row,
  same column, clamped if that row is shorter). All built on one shared
  row-layout function (`build_catalog_rows`) that rendering, navigation,
  and mouse hit-testing all reuse — so a keypress or a click can't
  resolve to a tile that isn't what's actually on screen, same principle
  as the pod-table hover fix from a few rounds back.
- [x] **Mouse**: hover or click resolves to a tile via `ui::tile_at`,
  which replays the exact same `Layout::horizontal` column split
  `draw_tiles_row` renders with.
- [x] **Auto-scroll-into-view**: moving the selection with the keyboard
  past the visible window scrolls just far enough to bring it back into
  view (`ui::scroll_to_show`) — mouse selection doesn't need this since
  you can only click what's already visible.
- [x] **Enter activates the tile** for the two kinds that already have a
  real list view — selecting "Pods" or "Deployments" and pressing Enter
  jumps there, same as picking them from the `m` menu. Every other tile
  (the ~20 count-only ones) does nothing on Enter yet, consistent with
  the earlier scope decision.
- [x] Unit-tested the grid math directly (6 new tests, 21/21 total) since
  it's exactly the kind of logic verifiable without a real terminal:
  row-wrap within a section, crossing into the next/previous section in
  both directions with column-matching, and clamping at the very first/
  last tile.
- [x] Build/tests/clippy all clean. Committed as its own unit and pushed.

## Done since last update (2026-09-17, later still) — every catalog kind now has a real list + spec view

Previously only Pods/Deployments had a list view; the other ~20 kinds
(ConfigMaps, DaemonSets, Secrets, Services, ClusterRoles, ...) were
count-only tiles that did nothing when selected. Per explicit request,
gave every one of them a working view.

- [x] **`k8s::CatalogKind` trait** — a type-erased handle (`count()`,
  `rows()`, `spec_at(index)`) over a live-watched resource kind, boxed as
  `Box<dyn CatalogKind>`. Lets `Catalog` hold ~20 different concrete `K`s
  in one `Vec` and treat them uniformly instead of a hand-written struct
  field + match arm per kind (the old `watch_count`/per-field approach
  from the previous round).
- [x] **Switched from polling to live watches** for these ~20 kinds —
  `watch_count`'s 15s-interval `.list()` poll is gone, replaced by
  `k8s::watch_generic`/`WatchedKind`, the same reflector pattern already
  used for Pods/Deployments/Nodes/Events. Counts, rows, and spec are now
  all real-time, not up to 15s stale.
- [x] **Nodes reuses the existing `node_store`** reflector (wrapped in
  `WatchedKind::from_store`) instead of opening a second watch on the
  same kind — the Cluster Issues panel and the Nodes list/spec view now
  share one underlying watch.
- [x] **Generic Namespace/Name/Age table** (`ui::draw_generic_table`,
  `Rows::Generic`) for every kind that isn't Pods/Deployments. Deliberate
  simplification: cluster-scoped kinds (Nodes, ClusterRoles, PVs,
  StorageClasses, ClusterRoleBindings) show "-" for namespace rather than
  getting their own column set — one shared table for ~20 kinds is worth
  the loss of kubectl's per-kind columns.
- [x] **`d` opens the real YAML spec** for any of these kinds, same
  collapsible-tree popup Pods/Deployments already use — `CatalogKind::
  spec_at` calls the same `manifest_value` used everywhere else, so
  managedFields-stripping etc. all just applies for free.
- [x] **Every Overview tile is now a real jump target** — `Enter` on any
  of the ~23 non-Overview tiles switches to that kind's list, not just
  Pods/Deployments. Implemented via `kind_for_label`, a label → 
  `ResourceKind` lookup (the join key between the (label, count) tuples
  the catalog renders and the enum `current_kind` switches on).
- [x] **`m` menu extended to all 25 kinds**, grouped into the same six
  sections as the Overview catalog (Cluster/Workloads/Config/Network/
  Storage/Access Control). Reworked `draw_menu_popup` to wrap tiles
  within a section (up to 7 in Workloads now) instead of dividing a
  fixed-height row evenly — at 7 tiles that would've been unreadably
  thin. Uses the same tile-width-based wrapping the Overview grid uses.
- [x] Verified against the live `knav-test` k3d cluster with a disposable
  scratch binary (not part of the repo): counts and row listings for
  Nodes, Namespaces, ReplicaSets, DaemonSets, ConfigMaps, and
  ClusterRoles all matched `kubectl get <kind> -A` exactly (1, 7, 4, 1,
  14, 76), cluster-scoped kinds correctly showed "-" for namespace, and
  `spec_at(0)` produced valid, sensible YAML for each.
- [x] Build clean, 21/21 tests pass (no new tests added — this round is
  data-layer generalization + wiring, not new logic worth unit-testing
  beyond what live-cluster verification already covered), clippy clean.

### Not done on purpose

- No specialized columns for any of the ~20 generic kinds (e.g. no
  "TYPE"/"DATA" for Secrets, no "PORTS" for Services) — out of scope for
  this round, which was about getting every kind *a* working view, not
  matching kubectl's per-kind column sets.
- No mouse row-selection or hover for the generic table (Deployments
  didn't have this either — parity with the existing non-Pods table, not
  a regression).
- Menu keyboard navigation is still flat Left/Right (h/l) across all 25
  tiles in registration order, not row-aware like the Overview grid's
  Up/Down — the Overview grid needed real 2D nav because you spend time
  browsing it; the menu is a quick switcher, not lingered on.

## Done since last update (2026-09-17, later still #2) — menu Up/Down bug fix + CRD support

Two separate rounds, both requested together: the `m` menu was stuck on
the top row (Up/Down never worked, contradicting the "not done on
purpose" note above — this fixes it), and CRDs (Flux, Helm charts, or in
this cluster's case Traefik Hub/Gateway API/k3s's own CRDs) weren't
supported at all.

### Menu Up/Down bug

- [x] The menu only ever handled `h`/`l` — `j`/`k`/arrows were silently
  ignored, so selection couldn't leave the top row of any section with
  more than one row of tiles (Workloads has 7, wraps to 2+ rows).
- [x] Fixed by making menu selection `(section, tile)` like the Overview
  grid, moved via a shared `ui::move_selection` both `move_tile_selection`
  (Overview) and the new `move_menu_selection` (menu) delegate to — one
  implementation of the wrap/cross-section rules instead of two.
- [x] Extracted the menu's layout into `menu_sections()`, used by both
  the render pass and the key handler so they can't diverge — same
  "render and navigate off the same function" principle as
  `build_catalog_rows` for the Overview grid.
- [x] `menu_position_for()` — opening the menu now starts on the
  currently-viewed kind instead of resetting to the first tile.
- [x] Removed `ResourceKind::ALL` — nothing needs a flat list of every
  kind now that the menu is driven by `menu_sections()` directly (this
  also removes the "the menu's tile count must equal ALL.len()" implicit
  invariant that a dynamic CRD list would've broken anyway).
- [x] 2 new tests (menu covers every kind exactly once, position lookup
  resolves correctly), 23/23 total, clippy clean.

### Custom Resource (CRD) support

Discussed the design tradeoff first: eagerly live-watching every
discovered CRD (like the ~20 built-in kinds) doesn't scale — a cluster
with Flux + cert-manager + Prometheus Operator etc. installed can easily
have 50+ CRDs, and this cluster alone already has 33 just from Traefik
Hub/Gateway API/k3s. Chose **lazy**: discover all installed CRDs once at
startup (cheap, one list call), but don't start watching a CRD's actual
objects until the user opens it.

- [x] `k8s::discover_crds` lists every `CustomResourceDefinition` once at
  startup and extracts group/kind/plural/a served version (preferring
  the storage version)/scope. `group`/`kind` are leaked to `&'static
  str` — a one-time, bounded-size leak — so a CRD kind can carry a plain
  `&'static str` label exactly like every built-in kind, no registry
  lookup needed just to render a title.
- [x] `ResourceKind::CustomResourceList` (a fixed sentinel, like
  `Overview`) is the "Custom Resources" picker: a GROUP/KIND/SCOPE table
  of every discovered CRD, no live data — just `discover_crds`'s output.
  `ResourceKind::CustomResource(index, label)` is one specific CRD kind's
  instances, `index` pointing into `Catalog`'s discovered list.
- [x] `Catalog::resolve()` is the one place that turns a `ResourceKind`
  into a live `CatalogKind` — for a `CustomResource`, it lazily spawns
  the watch on first access and caches it (`HashMap<usize, Box<dyn
  CatalogKind>>`); every later tick just reads the cached one. `Catalog`
  is now `&mut` in `run()` for this reason.
- [x] `k8s::WatchedDynamicKind` + `k8s::watch_crd` — a `DynamicObject`-
  backed watch (schema unknown at compile time, unlike every built-in
  kind), built from an `ApiResource` assembled from the CRD's group/
  version/kind/plural. `generic_row`/`snapshot_generic` had their
  `DynamicType = ()` bound relaxed to plain `Resource`, since
  `Resource::meta()` only reads `self` — this made them work for
  `DynamicObject` too, no separate CRD-specific row logic needed.
  `reflector::store()` needed swapping for `Writer::new(resource)` +
  `.as_reader()`, since it requires `DynamicType: Default` and
  `ApiResource` doesn't implement that (unlike `()`).
- [x] Enter on the "Custom Resources" tile/menu entry opens the picker;
  Enter on a row in the picker switches to that CRD's own generic
  Namespace/Name/Age list + `d`-to-spec view — same `ui::Rows::Generic`
  table every other generic kind already uses.
- [x] The Overview tile/menu entry for Custom Resources shows how many
  CRD *kinds* are installed (free — known from discovery, no watch
  needed), not a live object count — consistent with "list only until
  opened."
- [x] Verified against the live cluster (which turned out to already
  have 33 real CRDs — Traefik Hub, Gateway API, k3s's own) with a
  disposable scratch binary: `discover_crds` found all 33 with correct
  group/kind/version/scope, and watching `HelmChart` matched `kubectl
  get helmcharts -A` exactly (2 objects, same namespaces/names), with
  valid spec YAML.
- [x] Build clean, 23/23 tests pass, clippy clean.

### Not done on purpose

- No grouped/nested tile browser for CRDs by API group — the picker is
  one flat, sorted-by-group table instead. Simpler, and sorting already
  clusters same-group kinds next to each other visually.
- A CRD installed *while knav is running* won't appear until restart —
  discovery is a one-shot startup call, not polled. Installing a CRD is
  rare compared to objects of it coming and going, so this wasn't worth
  a periodic re-list.
- No specialized columns for CRD instances (same simplification as the
  other generic kinds) — Namespace/Name/Age regardless of what the CRD
  actually is.

## Done since last update (2026-09-17, later still #3) — real Kubernetes icons in the Overview grid

Overview tiles used our own emoji picks, not official icons — noted
explicitly in an earlier round as "our own choice, not official." Asked
about actually integrating the real ones; the constraint is that
ratatui renders text cells, not images, so this needed a terminal
graphics protocol, not just a different glyph. Discussed the tradeoff
(real images via Kitty/Sixel, falling back to a coarse halfblock mosaic
elsewhere, vs. staying with text glyphs) — went with real images, since
Ghostty (this machine's terminal) supports the Kitty protocol.

- [x] Vendored 26 SVGs from the official `kubernetes/community` icon set
  (`assets/icons/`, `unlabeled` variants — icon only, no text) covering
  every built-in `ResourceKind` plus a generic `crd.svg` for Custom
  Resources. Attribution + license terms (dual Apache-2.0/CC-BY-4.0,
  redistribution permitted) recorded in `assets/icons/ATTRIBUTION.md`.
- [x] `src/icons.rs`: rasterizes each SVG once via `resvg`/`usvg`/
  `tiny-skia` onto a 128×128 transparent canvas (scaled to fit, centered,
  aspect preserved), converts to an `image::DynamicImage`, and hands it
  to `ratatui-image`'s `Picker::new_resize_protocol` — cached per icon
  (`HashMap<&'static str, StatefulProtocol>`), built lazily the first
  time that resource kind's tile is actually drawn.
- [x] `IconCache::detect()` calls `Picker::from_query_stdio()` (queries
  the terminal's real capability via an escape sequence) once, after raw
  mode is enabled but before the event-read loop starts, so it can't
  race with crossterm's own stdin reads; falls back to
  `Picker::halfblocks()` (pure Rust, no querying) if detection errors.
- [x] `ratatui-image` pulled in with `default-features = false, features
  = ["crossterm"]` — its defaults require the system `chafa` library via
  pkg-config, which isn't installed here and shouldn't be a hard
  requirement for a hobby TUI; the built-in halfblocks/Kitty/Sixel
  encoders don't need it.
- [x] Grew each Overview tile from 5 to 7 rows tall (2 border + a 3-row
  icon area + count line + label line) to give the image real room;
  `TILE_HEIGHT`/`TILE_ICON_HEIGHT` are the only places this is defined,
  so the catalog scroll math, menu popup, and hit-testing all picked it
  up automatically.
- [x] Moved the Overview tile's label→`ResourceKind` mapping from a
  free function in `main.rs` (`kind_for_label`) to `ResourceKind::
  from_label`, next to the existing `label()`, since `ui::draw_tile` now
  also needs it (to know which icon to fetch) — one mapping instead of
  two.
- [x] When a modal is open (dimmed background), tiles still fall back to
  the plain glyph instead of a real image — a full-color image would
  keep reading as "in focus" even while everything else recedes for the
  modal, undermining the whole point of dimming.
- [x] Ran the release binary attached to a real pty for several seconds:
  no panic, and the terminal output showed genuine Kitty graphics
  protocol escape sequences being sent (`_Gi=...,a=q,t=d,f=24;...`),
  confirming `Picker::from_query_stdio` detected Kitty and the rendering
  pipeline is actually executing — this doesn't confirm the pixels look
  right (can't inspect rendered terminal output from here), so visual
  confirmation is still on the user.
- [x] Build clean, 23/23 tests pass (no new tests — this is a rendering
  pipeline change with no new pure logic beyond what integration/visual
  testing covers), clippy clean.

### Not done on purpose

- No per-frame background thread for image resize/encode
  (`ratatui_image::thread::ThreadProtocol`) — since each tile's Rect size
  is constant across frames, the resize/encode only actually runs once
  per icon (first render), not every frame, so the crate's own "don't
  block the UI thread" warning doesn't really bite here. Worth
  revisiting only if tiles ever become dynamically resizable.
- Menu popup tiles still use the old emoji glyphs, not real icons — this
  round only touched the Overview grid to keep the change bounded.
- No alpha/background-color tuning for the halfblocks fallback path
  (non-Kitty/Sixel terminals) — whatever `ratatui-image` does by default.

## Done since last update (2026-09-18) — Esc/`:` nav (previous session) + collapsible, centered Overview

Two rounds of feedback in one: (1) Esc quitting instead of backing out,
and wanting `:pods`/`:q` k9s-style commands — already implemented and
pushed in the prior session; and (2) polish on the Overview grid: center
text/tile-rows instead of packing them flush-left, and make sections
collapsible like the spec tree.

### Overview: centered layout + collapsible sections

- [x] **Tile rows are now centered** instead of packed flush-left with
  the leftover space trailing on the right — `tile_row_constraints()`
  puts a `Fill(1)` on both sides of the tile group instead of only a
  trailing filler. One function shared by `draw_tiles_row` (render) and
  `tile_at` (mouse hit-testing), same "can't drift apart" principle as
  everywhere else in this file.
- [x] **Section headers are centered dividers** now:
  `───── ▾ Title ▾ ─────`, filling the row width — replacing the old
  flush-left `── Title`. The `▸`/`▾` indicator mirrors the spec tree's
  own collapsed/expanded convention.
- [x] **Gauge titles centered** (`CPU: ... / ...` etc.) — were left-
  aligned by ratatui's `Block` default.
- [x] **Sections are now collapsible**, "like the spec tree": `Tab`
  toggles the section the current tile selection is in; clicking
  directly on a section header (mouse) also toggles it. Works for all 7
  Overview sections *and* "Cluster Issues".
- [x] `CatalogRow::SectionHeader` now carries a section index + collapsed
  flag; `build_catalog_rows` omits a collapsed section's `Tiles`/`Issue`
  rows entirely rather than just visually hiding them.
- [x] Collapsed sections are fully transparent to keyboard navigation,
  not just visually hidden — `move_selection` now skips over *any
  number* of consecutive empty/collapsed sections in one keypress
  (`next_nonempty_section`/`prev_nonempty_section`), rather than landing
  on a dead "phantom" tile inside a collapsed section and needing a
  second keypress to escape it.
- [x] New `ui::header_at` (mirrors `tile_at`'s row-walking) for mouse
  click-to-toggle.
- [x] 2 new tests (a collapsed section is skipped by one `Right` press
  across multiple sections; `build_catalog_rows` omits a collapsed
  section's tiles), 28/28 total, clippy clean.
- [x] Ran the release binary on a real pty against the live cluster:
  runs cleanly for several seconds, no panic. Can't inspect rendered
  terminal output from here, so the actual look (centering, dividers,
  collapse) still needs the user's own `cargo run`.

### Not done on purpose

- No outer bordered box around the whole catalog/section area — kept the
  existing "just tiles get boxes" look, since spanning a real border
  across a variable-height, independently-scrolled set of rows would've
  been a much bigger rendering change for a "pretty" label the divider-
  style headers plus already-rounded tiles arguably already cover.
- Menu popup tiles are unaffected by any of this (no collapse, no
  centering changes there) — this round was scoped to the Overview grid.

## Done since last update (2026-09-18, later) — Freelens-style node drill-down

Asked for cross-type integration: clicking a Node should show what's
running on it, with usage gauges, "like in Freelens" — and be able to
drill from there into a pod's containers, same as from the Pods list.

- [x] `metrics::watch_node_metrics` now publishes a per-node breakdown
  (`ClusterUsage::nodes: Vec<NodeUsage>`, `ClusterUsage::for_node(name)`)
  alongside the existing cluster-wide total — same poll, no extra API
  calls, metrics-server already returns per-node data that was
  previously only being summed.
- [x] `k8s::node_capacity(node)` — one node's own CPU/Memory/Pods
  allocatable, mirroring `node_allocatable_sum` but for a single node
  instead of the Overview's cluster-wide sum.
- [x] New `Overlay::NodeDetail`: that node's CPU/Memory/Pods gauges
  (reusing the exact `draw_gauge` the Overview panel uses) above the
  pods actually scheduled on it (reusing the exact pod table the Pods
  list view uses — container dots included). "metrics unavailable"
  fallback if metrics-server isn't installed, consistent with the
  Overview panel's own fallback.
- [x] `Enter` on a Node (from the Nodes list) opens it; `j`/`k` move
  within its pods, `Enter` on one of those opens Containers → Logs
  exactly like from the main Pods list, `d` opens the Node's own YAML
  spec, `Esc`/`q` closes back to the Nodes list.
- [x] `Mode::Containers` gained a `back: Box<Mode>` field (mirroring the
  one `Mode::Logs` already had) so `Esc` from a pod's containers
  correctly returns to wherever you actually opened it from — the Pods
  list normally, or `NodeDetail` when opened from there. Threaded
  through the Containers→Logs transition too (which snapshots the
  Containers view to return to, and that snapshot needed its own `back`
  now as well).
- [x] Verified against the live cluster with a disposable scratch
  binary: per-node metrics matched `kubectl top nodes` exactly (71m
  cpu / ~964MB), node capacity was sane (2000m/2GB/110 pods), and pod-
  to-node filtering correctly found all 16 pods scheduled on the node
  while excluding the one genuinely unscheduled pod (`shop/oversized`,
  Pending).
- [x] Build clean, 28/28 tests pass, clippy clean. Ran the release
  binary on a real pty for several seconds with no panic.

### Not done on purpose

- No mouse support inside the NodeDetail popup (no hover, no click-to-
  select) — keyboard-only, consistent with the existing Containers
  popup which also has no mouse handling.
- No node-level Warning/condition display duplicated inside NodeDetail
  itself — that's still only in the Overview's Cluster Issues panel.

## Done since last update (2026-09-18, later still) — CRD groups + interface consistency pass

Asked for two more things: organize "Custom Resources" by API group
instead of one flat tile/list, and make the interface generally cleaner
— specifically calling out inconsistent corners across boxes.

### CRD groups

- [x] `ResourceKind::CustomResourceGroup(&'static str)` — same shape as
  `CustomResource`'s label-carrying pattern (the group string is already
  `&'static str`, leaked once at discovery, so no registry lookup
  needed). `CustomResourceList` still means "everything, unfiltered."
- [x] The Overview's "Custom Resources" section now shows one tile per
  discovered API group (this cluster: `gateway.networking.k8s.io`,
  `hub.traefik.io`, `traefik.io`, `helm.cattle.io`, `k3s.cattle.io`)
  alongside the original "Custom Resources" tile for the flat/unfiltered
  view — verified the group/count breakdown against a disposable scratch
  binary using the real discovery code, matched a `kubectl get crds`
  cross-check exactly (5 groups: 6/13/10/2/2).
- [x] `Catalog::kind_for_tile_label` — the Overview tile click-through
  now tries the fixed kinds first (`ResourceKind::from_label`) and falls
  back to matching a discovered CRD group, since group names can't be
  known by the static resolver ahead of time.
- [x] `Rows::CrdList` now carries `(real_index, CrdInfo)` pairs instead
  of bare `CrdInfo` — necessary once the picker can show a *filtered*
  subset, so picking a row can still open the right kind out of the
  *full* discovered list, not the filtered one's own position. Also
  carries a heading string so the title reads "Custom Resources" for
  the flat view or the group name for a filtered one.
- [x] `Esc` from a specific CRD kind's instances now returns to the
  group it actually came from (or the flat list, if for some reason
  that group can't be found), not always the flat list.

### Interface consistency

- [x] Several boxes had sharp corners while everything else was already
  rounded (tiles, the metrics panel, NodeDetail) — the Pods/Deployments/
  generic/CRD-picker tables, the container-hover popup, and the Spec/
  Containers/Logs popups all get `BorderType::Rounded` now too. Every
  bordered box in the app is rounded.
- [x] Tile labels are truncated with an ellipsis instead of getting cut
  off raw mid-character — matters more now that CRD group names
  (`gateway.networking.k8s.io`) are long enough to actually hit the
  tile's fixed width, using the same `truncate()` helper already used
  for long warning messages.
- [x] CRD-group tiles now show the real CRD icon (not a "no icon"
  glyph) — `draw_tile` recognizes any label under the "Custom Resources"
  section that isn't a fixed kind as a CRD group and resolves its icon
  accordingly, since that's the only place such dynamic labels appear.
- [x] Build clean, 28/28 tests pass, clippy clean. Ran the release
  binary on a real pty against the live cluster for several seconds,
  no panic.

### Not done on purpose

- The `m` menu's "Custom Resources" entry still opens only the flat,
  unfiltered list — it doesn't grow group sub-tiles the way the Overview
  grid does. `MenuSection.tiles` is a `&'static [ResourceKind]` built
  once at compile time; giving it a dynamically-sized, per-run list of
  discovered groups would need a real restructuring (owned `Vec`s with
  their own lifetimes threaded through `menu_sections()`) for a
  secondary surface — the Overview grid was the explicit ask.
- No further "prettier" tuning beyond corner consistency + label
  truncation (e.g. tile background tinting) — real terminal-graphics
  images (Kitty protocol) composite as their own layer on top of the
  terminal grid, so a tile background color wouldn't visually apply to
  the icon area anyway, and guessing further polish without being able
  to see the actual rendering risked making things worse, not better.

## Done since last update (2026-09-18, later still #2) — un-pin the metrics bar, redo CPU/Mem visuals, live node usage in the list

Three complaints in one message: the "top bar" (Cluster Resources
panel) shouldn't be permanently pinned above the scroll, the CPU/Memory
representation itself looked bad, and Node usage should be visible in
the Nodes list itself, not just after pressing Enter. Also asked for
bigger, rounder tiles.

### Metrics: no longer pinned, and redrawn

- [x] The metrics panel was a fixed `Constraint::Length` chunk sitting
  above a separately-scrolled catalog area — now it's just the first
  entry in the same scrollable row list everything else uses
  (`CatalogRow::Metrics`), under its own "Cluster Resources" divider
  header, collapsible exactly like every other section. `catalog_area`
  now just returns the full frame — there's no separate pinned region
  left to carve out.
- [x] Replaced ratatui's `Gauge` widget with a hand-built single-line
  meter (`draw_meter`): `CPU     ▓▓▓▓▓▓░░░░░░░░░░░░░░░░  71m / 2000m (3%)`.
  The actual problem with the old version: `Gauge` bakes in its own
  centered percentage label with no way to turn it off except by also
  losing the ability to show real used/capacity numbers — our own title
  text and the gauge's own auto-label ended up overlapping/duplicating.
  The new meter is one clean line, bar width adapts to whatever space is
  available, colored green/yellow/red by usage same as before.
- [x] Reused `draw_meter` for the NodeDetail popup's gauges too (was
  still on the old `Gauge`-based `draw_gauge`, now deleted along with
  the `Gauge` import) — one consistent representation everywhere CPU/
  Memory shows up.

### Nodes list shows usage without opening a node

- [x] `k8s::NodeRow`/`k8s::node_row()` — Nodes now get their own
  specialized table (`ResourceKind::Nodes` → `Rows::Nodes`, not the
  generic Namespace/Name/Age one) with NAME/STATUS/CPU/MEMORY/PODS/AGE
  columns. CPU/MEMORY show a compact inline bar (`usage_bar`, 10 chars
  wide) right in the list — no need to press Enter first anymore.
  STATUS reflects the real `Ready` node condition (green/red).
  `usage_bar` shows `n/a` when metrics-server isn't installed.
  Sourced from `k8s::snapshot_generic(node_store)` — the exact same
  sorted list the generic Nodes catalog entry already uses internally —
  so the new specialized rows stay index-aligned with `generic_rows`
  for the existing `d` (spec) key, no changes needed there.
- [x] Verified against the live cluster with a disposable scratch
  binary: Ready status, ~47% memory (matched `kubectl top nodes`'s
  46%), and 16/110 pods all correct.

### Bigger tiles

- [x] `TILE_WIDTH` 18→22, `TILE_HEIGHT` 7→8, `TILE_ICON_HEIGHT` 3→4 —
  more room overall, and a genuinely bigger icon too since
  `centered_square` scales with `TILE_ICON_HEIGHT`, not just a bigger
  empty box around the same size image.
- [x] Worth being upfront about a real constraint: a terminal is a
  monospace character grid, and `BorderType::Rounded` only swaps the
  four corner *characters* (`┌┐└┘` → `╭╮╰╯`) — there's no way to get a
  larger-radius "chunky" rounded corner the way a GUI can, no matter how
  big the box is. Bigger tiles make the existing rounding read as more
  deliberate/proportionate, but the corners themselves are still just
  one character each.
- [x] Build clean, 29/29 tests pass (2 new, for the metrics section's
  own collapse behavior), clippy clean. Ran the release binary on a
  real pty against the live cluster for several seconds, no panic.

### Not done on purpose

- No sparkline/history graph for CPU/Memory (a single current-value
  meter only) — would need to start retaining a rolling sample buffer
  over time, a real feature on its own rather than a visual tweak.
- Node's own Warning conditions aren't shown inline in the Nodes list
  (only STATUS: Ready/NotReady) — the detailed condition messages are
  still only in Overview's Cluster Issues panel and the node's own spec.

## Done since last update (2026-09-18, later still #3) — fixed a real scroll bug, plus two polish requests

Reported "Custom Resources isn't showing anymore." Verified the data
layer first with a temporary debug hook (dumped `Catalog::sections()`'s
actual output to a log file, ran against the live cluster, confirmed
all 6 tiles — the flat list plus 5 groups — were present with correct
counts, then removed the hook). The bug wasn't data, it was navigation:
**mouse wheel scrolling was never wired up for the Overview page at
all** — only click-to-select and hover. Between last round's bigger
tiles and the metrics panel now living inside the scroll instead of
being pinned, "Custom Resources" (near the bottom) now needs
meaningfully more scrolling to reach, and the most natural way to get
there — the mouse wheel, especially after asking for "just a scrollable
page" — silently did nothing.

- [x] Wired `MouseEventKind::ScrollDown`/`ScrollUp` for the Overview
  page: adjusts `overview_scroll` directly (independent of tile
  selection), clamped to the real content length via the new
  `ui::catalog_row_count`. Keyboard navigation (`j`/`k`/arrows, which
  auto-scroll via `scroll_to_show`) still works exactly as before —
  this was a pure gap, not something that regressed.
- [x] Menu tile selection: a colored border alone read as too subtle to
  actually notice — the selected tile in the `m` menu now gets a solid
  filled cyan background instead, unmistakable regardless of terminal
  theme. (Scoped to the menu specifically, since that's what was asked
  — the Overview grid's own tile selection keeps its existing border+
  text-color treatment.)
- [x] The generic Namespace/Name/Age table now drops the NAMESPACE
  column entirely when every row is cluster-scoped (Nodes, ClusterRoles,
  PVs, StorageClasses, cluster-scoped CRDs, ...) instead of showing a
  column that's all dashes — `any_row_has_namespace`, unit-tested.
- [x] Build clean, 31/31 tests pass (2 new), clippy clean. Ran the
  release binary on a real pty against the live cluster for several
  seconds, no panic.

### Not done on purpose

- Mouse wheel scroll wasn't added to the other list views (Pods/
  Deployments/Nodes/generic/CRD tables) — those rely on keyboard
  j/k + ratatui's own `Table` auto-scroll, which already works; this
  round was specifically about the Overview page's gap.
- Overview grid's own tile selection wasn't changed to a filled
  background — only the `m` menu was, per what was actually asked.

## Done since last update (2026-09-18, later still #4) — Overview rebuilt as Miller-columns, fixed dashboard header

A genuine redesign, not a tweak: replaced the flow-wrapping tile grid
entirely with a Miller-columns-style browser (one column per category —
Cluster, Workloads, Config, ...— each listing its kinds vertically,
horizontal scroll between columns), plus a fixed-size dashboard strip
(CPU/Memory/Pods meters + a capped Cluster Issues) that no longer
scrolls away at all. Discussed the tradeoff against a smaller
incremental fix first (selectable headers + collapse-all on the
existing vertical grid) — the user chose the full column rewrite.

While investigating this, found and fixed a **real pre-existing bug**:
`row_index_of_tile` (keyboard auto-scroll) never accounted for the
Metrics section's rows after it was added two rounds ago — meaning
keyboard navigation toward later sections was under-scrolling too, not
just the mouse wheel gap fixed last round. This whole class of bug is
now structurally impossible: the old approach re-derived row offsets by
hand in three different places (`row_index_of_tile`, `scroll_to_show`,
`tile_at`/`header_at`) and they drifted out of sync; the new design has
no equivalent "recompute the same count a different way" step to drift.

- [x] **Fixed dashboard header** (`draw_top_panel`, `top_area_height`):
  CPU/Memory/Pods meters, then Cluster Issues capped at
  `MAX_VISIBLE_ISSUES` (5) with a "… and N more" line — always visible,
  bounded height, not part of any scroll or collapse system anymore
  (collapsing was only useful when this was taking a lot of scroll
  space; now it's small and fixed, so it just stays visible).
- [x] **Columns replace the tile grid** (`draw_columns`/`draw_column`/
  `draw_column_item`): one column per catalog category, each a header
  (`▾`/`▸`, same convention as the spec tree) plus its kinds listed
  vertically as compact single-line items (small icon + name + count).
  `OverviewSelection::{Header, Item}` replaces the old `(usize, usize)`
  tile coordinate.
- [x] **Headers are directly selectable and toggleable** — `Up` from a
  column's first item lands on its own header; `Down` from a header
  enters its first item (or does nothing if collapsed/empty); `Enter`
  on a header toggles it, same as `Tab` already did indirectly.
  `Left`/`Right` move directly between columns, landing on the target
  column's header instead of a nonexistent item if it's collapsed.
- [x] **`z`/`Z` collapse all / expand all** columns at once.
- [x] **Continuous horizontal scroll**: `Left`/`Right` past the visible
  edge scrolls the column view via `scroll_columns_to_show`, the
  horizontal analog of the old vertical auto-scroll. Mouse click
  resolves to a header or item via `column_hit`, replacing the old
  `tile_at`/`header_at` pair.
- [x] Per user's mid-turn note: made icons short (one row tall) and
  items compact (one line each: small icon + bold name + count) instead
  of the earlier idea of 2-row items — more kinds visible per column
  without scrolling, at the cost of the icon being quite small (a real
  Kitty-protocol image at 1×3 cells will look tiny, not detailed — an
  honest tradeoff for density over icon fidelity).
- [x] Removed the entire old tile-grid stack now that nothing uses it:
  `CatalogRow`, `build_catalog_rows`, `move_tile_selection`,
  `row_index_of_tile`, the old vertical `scroll_to_show`, `tile_at`,
  `header_at`, `draw_catalog`, `draw_section_header`, `draw_tiles_row`,
  `draw_tile`, `TILE_HEIGHT`/`TILE_ICON_HEIGHT`. `move_selection`/
  `next_nonempty_section`/`prev_nonempty_section`/`move_menu_selection`
  were kept — they're shared with (and still used by) the `m` menu
  popup's own unrelated tile grid.
- [x] Rewrote the whole test module for the new model (13 tests:
  within-column movement, header enter/exit transitions, collapsed-
  column handling, cross-column Left/Right including landing on a
  collapsed target's header, horizontal scroll-into-view, mouse hit-
  testing) — 33/33 total, clippy clean.
- [x] Verified with real interactive input (not just a static render):
  used `expect` to drive the actual release binary through a pty —
  column navigation (Left/Right/Up/Down), Tab, `z`/`Z`, scrolling across
  many columns, and the full Node → NodeDetail → Esc → Esc → quit chain
  — all against the live cluster, no panics, clean exit both times.

### Not done on purpose

- No per-column vertical scrolling — the widest column (Workloads, 7
  items) fits comfortably in a normal terminal height at 1 row/item, so
  this wasn't needed for the current catalog's realistic sizes. Would
  need revisiting if a category ever grew much larger.
- The Cluster Resources/Issues dashboard strip lost its own collapse
  toggle (it had one two rounds ago) — now fixed-size and always shown,
  per this round's explicit request.

## Done since last update (2026-09-18, later still #5) — bordered item cards, cluster picker, Node parity pass

### Overview: rounded-border item cards (herdr-style selection)

- Every column and every item is now its own `BorderType::Rounded` box
  instead of a plain bg-color fill on selection — selecting a header or
  item highlights the *whole box's border*, herdr-style, rather than just
  tinting a background. Corners are now unambiguously rounded everywhere
  (previously the Overview had no border chars at all on items, just a
  flat fill).
- An item's name is now its box's *border title* — bigger and more
  prominent than a cramped inline span competing with an icon on a
  single-cell-tall row. The content row inside the box now just holds the
  icon + live count.
- Item cards are 3 rows tall now (was 1), so a column with many kinds
  (Workloads, 7) can't always fit on screen at once — added a vertical
  item-scroll for whichever column holds the current selection (mirrors
  the existing horizontal column-scroll: same `scroll_columns_to_show`
  function, reused, not duplicated). Verified with a 60×20 pty session —
  no panics even at that size.
- `COLUMN_WIDTH` bumped 26→28 and a 1-cell gap added between columns
  (`visible_columns`/`column_hit`/`draw_columns` all updated to agree on
  the same gap math) so columns read as distinct panes, not
  edge-to-edge boxes.

### A freelens-style cluster picker, `--context`, and startup config

- New full-screen picker (`picker.rs`) shown before knav connects to
  anything: every kubeconfig context, live-filtered by fuzzy match as you
  type, Enter to connect, Esc/Ctrl-C to quit. Off by default — controlled
  by a new `[startup] mode = "direct" | "menu"` config key (`direct`,
  k9s's own default, connects straight to whatever `kube` infers; `menu`
  always shows the picker first).
- New `-c`/`--context <name>` CLI flag (hand-rolled parsing — one flag
  doesn't justify a full arg-parsing crate yet) fuzzy-matches the given
  name against the kubeconfig's contexts and connects to the best match
  directly, non-interactively, regardless of `startup.mode`.
- New dependency-free fuzzy matcher (`fuzzy.rs`) shared by both the flag
  and the picker's own type-to-filter search, so typing the same string
  in either place resolves to the same context.
- Verified against the live cluster: `--help`, an unmatched `-c` query
  (clean error + exit 1, no panic), a fuzzy `-c` match connecting
  straight through, and the interactive picker's filter/select and
  cancel paths, all via `expect`-driven pty sessions.

### Node field completeness (Freelens parity pass, Nodes only)

- Nodes list gained ROLES and VERSION columns (kubectl's `-o wide`
  convention); STATUS now appends ",SchedulingDisabled" for a cordoned
  node instead of needing a separate column, kubectl's own convention.
- The node drill-down (Enter on a node) gained a new info panel above the
  pod table: schedulability (cordoned or not), roles, kubelet version,
  internal/external IP, OS image, kernel version, container runtime, the
  *full* condition list (healthy conditions included — deliberately not
  filtered to problems only, unlike the Cluster Issues panel, since this
  is a diagnostic detail view), and any taints.
- Verified against the live cluster via `tmux capture-pane` (renders the
  real terminal screen as plain text, unlike raw pty log capture which is
  full of cursor-positioning escapes) — confirmed ROLES/VERSION show in
  the list and the full info panel + conditions render correctly for the
  real k3d node.

### Not done on purpose

- The "check every resource kind for missing fields" ask was scoped to
  Nodes only this round (the explicit example given). Pods/Deployments/
  Services/etc. likely have similar gaps against Freelens (e.g. Services
  missing TYPE/CLUSTER-IP/PORTS, Pods missing QoS class) — not audited
  yet, flagged as a follow-up below.
- No ambiguity handling for `--context` when multiple contexts score
  equally under fuzzy match — picks whichever the scorer or iteration
  order happens to return; fine for realistic kubeconfig naming, could
  bite someone with near-duplicate context names.

## Done since last update (2026-09-18, later still #6) — Resources/Events boxes, broadened feed, filterable browser, collapse removed

### Dashboard: renamed, boxed, colored

- "Cluster Resources" -> "Resources", "Cluster Issues" -> "Events" —
  both now their own rounded-border box, same visual language as the
  column boxes below (per explicit request: "do one for the Events and
  Resources at the top"). Selecting either (keyboard or mouse) highlights
  its whole border, herdr-style; otherwise the Events box's border
  reflects overall cluster health at a glance — green (no warnings),
  yellow (a warning exists), red (a Node-condition warning exists,
  since that affects everything scheduled on it).

### Events feed broadened from warnings-only to everything, chronologically

- `k8s::Warning` renamed/widened to `EventEntry` with a `severity`
  (`Normal`/`Warning` — the only two Kubernetes itself defines, so no
  fabricated third "Errors" bucket). The feed now includes every cluster
  Event, not just Warning-type ones, plus each node's own problem
  conditions folded in as synthetic Warning entries — same chronological
  sort as before.
- Dashboard preview line gained a TYPE column and Normal-severity color
  (a muted green, reads as "routine" next to Warning's yellow and a
  Node-warning's red).

### A full, filterable Events browser

- Pressing Enter while the Events box is selected opens a new
  full-screen browser (`Mode::Events`/`Overlay::Events`) — every event,
  uncapped (unlike the 5-line dashboard preview), with `a`/`w`/`n`
  keyboard shortcuts to filter to all/warnings/normal. Table columns:
  TYPE, REASON, OBJECT, KIND, MESSAGE, AGE.

### Per-column collapse removed, replaced with scroll arrows

- Asked the user directly whether collapse (Tab/z/Z, the ▾/▸ header
  indicator) was still worth keeping now that columns are boxed
  side-by-side — their answer: remove it, since collapsing one column
  doesn't free space for the others in this layout (unlike the old 1-row
  design), so it wasn't earning its keybindings/visual noise anymore.
  Removed entirely: the `overview_collapsed: HashSet<usize>` state, Tab/
  z/Z key handling, the indicator glyph, and `collapsed` parameters
  threaded through `move_overview_selection`/`column_hit`/`draw_columns`/
  `draw_column`.
- Replaced with "◀"/"▶" arrows in 1-cell gutters flanking the columns
  area, shown only when scrolling that direction would actually reveal
  another column — the intended replacement affordance for "there's more
  here," suggested by the user themselves.

### Resources/Events are now part of Overview's keyboard/mouse navigation

- `OverviewSelection` gained `Resources`/`Events` variants sitting above
  the column grid in the Up/Down chain: Up from any column header lands
  on Events, Up from Events lands on Resources, Down reverses it. Left/
  Right are no-ops on both (nothing beside them to move to). Mouse clicks
  on either box select it the same way clicking a column header does.

### Verified against the live cluster

- `tmux capture-pane` screenshots (not just pty logs, which are full of
  cursor-positioning escapes and unreadable) at three terminal sizes
  (240x55, 140x45, 60x20/50x15 for a no-panic stress check) confirmed:
  the boxed Resources/Events panels render correctly with real data,
  horizontal column scrolling and the new ◀/▶ arrows work, Up/Down
  correctly walks Resources -> Events -> column headers and back, Enter
  on Events opens the browser showing all 11 real events with correct
  TYPE/REASON/OBJECT/KIND/MESSAGE/AGE, and `a`/`w`/`n` filtering narrowed
  the list correctly (11 -> 2 for warnings, 11 -> 9 for normal). No
  panics at any tested size. 42/42 unit tests, clippy clean.

### Not done on purpose

- No "Errors" filter/severity in the Events browser — Kubernetes Events
  only have `Normal`/`Warning` as a `type`; a substring heuristic on the
  message (like the log viewer's error/warn keyword coloring) could
  fabricate one, but that's guessing at intent rather than reflecting
  real API structure, so it was left out.
- No ambiguity/tie-break handling if a future column ever wants both
  Resources/Events-style navigation *and* per-item vertical scroll
  bookkeeping across a Resources<->Events switch — not needed yet since
  neither box has its own scrollable list.

## Done since last update (2026-09-18, later still #7) — Resources/Events enterable, default cursor, icon/count swap, a real back-chain bug found

- Overview now starts with `OverviewSelection::Resources` selected
  instead of the first column's first item — per explicit request that
  the cursor should land at the top of the page, not down in the
  catalog.
- Enter on Resources opens a new `Mode::ResourcesDetail`: full-size
  cluster CPU/Memory/Pods gauges plus a per-node usage table (reuses
  `draw_metrics_lines`/`draw_nodes_table` as-is, just given more room)
  — Enter on a node there drills into the existing `NodeDetail` view.
- Enter or a mouse click on a row in the Events browser opens that
  event's full, untruncated detail (`Mode::EventDetail`) — the browser's
  own MESSAGE column clips long text to fit the table width, so this is
  the "view it properly" the user asked for.
- Swapped each item card's icon and count position: count now sits in
  the flexible left zone, icon in the small fixed-width zone on the
  right (previously the reverse) — a literal "switch it around" per the
  explicit request.

### A real bug found while testing the above

- `NodeDetail`'s Esc handler was hardcoded to `Mode::List`, which was
  fine when reached the original way (from the Nodes list, where List +
  current_kind=Nodes is exactly right) but wrong when reached via the
  new Resources detail view — Esc there dropped straight to the
  Overview, and a *second* Esc then hit Overview's own "Esc quits"
  binding and killed the app. Fixed by giving `NodeDetail` a `back:
  Box<Mode>` pointer, the same pattern `Containers`/`Logs` already use,
  threaded through both places it's now opened from (the Nodes list and
  Resources detail) and through the Containers-from-NodeDetail chain.
- Caught this by actually testing the full navigation chain interactively
  via `tmux send-keys`/`capture-pane` rather than just confirming each
  new screen renders in isolation — worth calling out since it's exactly
  the kind of bug that only shows up when you follow a real user path
  (Overview -> Resources -> a node -> back -> back) rather than testing
  each new mode as a dead end.

### Verified against the live cluster

- Full `tmux` interactive verification: cursor starts on Resources;
  Enter opens the Resources detail with real gauges + the one real node;
  Enter on that node opens NodeDetail; Esc returns to Resources detail
  (not Overview); Esc again returns cleanly to Overview with the app
  still running. Separately: Events -> Enter opens the browser -> Enter
  on a row opens the full untruncated message -> Esc returns to the
  browser with its filter/scroll state intact. 46/46 unit tests, clippy
  clean.

## Open questions / next steps

- [ ] Audit Pods/Deployments/Services/ConfigMaps/Secrets/etc. against
      Freelens the same way this round did for Nodes — likely gaps:
      Services (TYPE, CLUSTER-IP, EXTERNAL-IP, PORT(S)), Pods (QoS class,
      pod IP), Deployments (strategy, selector). "Roles is just an
      example" was the user's own framing — Nodes was only the first pass.

- [ ] **Human: run `cd ~/Desktop/Work/knav && cargo run` in a real
      terminal against the still-running `knav-test` k3d cluster and
      confirm the table actually looks right.** Good things to check
      given the dummy data now in place: does `staging/flaky` really show
      green despite crash-looping (confirming the gap above), do colors
      look right for Pending/Failed, does the restarts column update live
      as `flaky` keeps restarting.
- [ ] Dig into `kdash` specifically — it's the highest-starred Rust entry
      and we haven't checked its navigation model yet. Could already cover
      what we think is open ground.
- [ ] Decide: is this project justified by (a) the hierarchy-nav +
      diagnosis combo being a genuine unmet niche, or (b) purely as a
      Rust/ratatui/kube-rs learning project regardless of competition?
      Be honest about which — it changes how much the "differentiator"
      needs to be airtight before starting.
- [ ] Design the actual navigation model — Miller columns (like `lfk`) is
      the known-working pattern, but we should decide if we're copying that
      structure or trying something else, given `lfk` already does Miller
      columns well. Right now v0.1 has no navigation at all — it's a
      single flat pod table across all namespaces, nothing more.
- [ ] Decide how "diagnosis" actually surfaces in the UI (a dedicated
      pane? inline annotations on the resource list? something like k9s's
      `:pulse` but with causes attached?)
- [ ] Replace the one-shot `r`-to-refetch with a live kube-rs
      watcher/reflector so the view updates in real time
- [ ] Support more resource kinds than just Pods
- [ ] Namespace filtering (currently always all-namespaces)
