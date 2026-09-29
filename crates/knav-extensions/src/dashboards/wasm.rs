//! WASM-backed dashboards: the code-carrying alternative to `declarative`'s
//! widgets, for a `wasm_dashboard` manifest (see
//! `extensions::manifest::ExtensionMeta::wasm_dashboard`). The whole API a
//! guest gets is `wit/dashboard.wit` — one export, `lines`, zero *callable*
//! imports — so this module is also where that boundary is actually
//! enforced: `run`, below, never registers a single function on its
//! `Linker`, which is what makes "no way to reach a client, a file, or the
//! network" true of the compiled code itself, not just of what this module
//! happens to wire up. (A `use`d type from `wit/dashboard.wit`'s own
//! `types` interface shows up as an "import" too, in component-model terms
//! — see `is_callable` — but it's a type id, not something invocable; the
//! empty `Linker` is what actually matters.)

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line as UiLine, Span as UiSpan};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use knav_common::theme::theme;

use super::context::DashboardContext;

wasmtime::component::bindgen!({
    path: "wit/dashboard.wit",
    world: "dashboard",
});

/// Fuel and memory budget for one `lines()` call — generous for real
/// dashboard logic over a cluster's worth of objects, bounded so a runaway
/// or hostile guest can't hang the render loop (fuel, which meters
/// instructions) or exhaust host memory (the limiter — a second, independent
/// cap, since fuel says nothing about what one instruction allocates).
const FUEL_BUDGET: u64 = 200_000_000;
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// One process-wide engine. Fuel metering is an `Engine`-level setting (via
/// `Config`), so it can't be turned on per call — every `Store` built from
/// this engine is fuel-bounded by construction.
pub fn engine() -> Result<Engine, String> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    Engine::new(&config).map_err(|e| e.to_string())
}

/// One extension's compiled dashboard. Compilation — the expensive part —
/// happens once, at `Registry::load` time; running it (`call`, below)
/// happens fresh every frame the dashboard is on screen, the same
/// "derive everything fresh" pattern the rest of `derive.rs` already
/// follows for native and declarative dashboards alike.
#[derive(Clone)]
pub struct WasmDashboardModule {
    component: Component,
}

impl WasmDashboardModule {
    /// Compiles `path` — already resolved, size-capped, and proven to sit
    /// inside its own extension's directory by
    /// `extensions::resolve_wasm_dashboard` before this is ever called.
    /// Never panics: a malformed or incompatible file is a plain `Err`,
    /// exactly like a bad TOML manifest is a plain `Err` from
    /// `Manifest::parse` — one bad extension never stops the others from
    /// loading (see `Registry::load`).
    pub fn compile(engine: &Engine, path: &std::path::Path) -> Result<Self, String> {
        let component = Component::from_file(engine, path).map_err(|e| format!("compiling {}: {e}", path.display()))?;
        Self::from_component(engine, component, &path.display().to_string())
    }

    /// The bytes-based twin of `compile`, for tests that already have a
    /// compiled fixture in memory (`include_bytes!`) rather than a path on
    /// disk.
    #[cfg(test)]
    fn compile_bytes(engine: &Engine, bytes: &[u8], label: &str) -> Result<Self, String> {
        let component = Component::new(engine, bytes).map_err(|e| format!("compiling {label}: {e}"))?;
        Self::from_component(engine, component, label)
    }

    /// Refuses anything a guest could actually *call* to reach out — a
    /// function, a core module, or a nested component. A `use`d type (say
    /// `kind-data`, shared structurally between `wit/dashboard.wit`'s world
    /// and its `types` interface) shows up as an "import" too, since the
    /// component format has no other way to reference a type defined
    /// elsewhere, but it carries no capability: there's nothing to invoke,
    /// only a type id. Rejecting every import outright would make a
    /// perfectly inert component (like the worked example) fail to load, so
    /// this looks at *what kind* of import each one is instead of just
    /// counting them.
    fn from_component(engine: &Engine, component: Component, label: &str) -> Result<Self, String> {
        for (name, ext) in component.component_type().imports(engine) {
            if is_callable(engine, &ext.ty) {
                return Err(format!("{label} imports \"{name}\", which a dashboard component must never do"));
            }
        }
        Ok(Self { component })
    }
}

fn is_callable(engine: &Engine, item: &wasmtime::component::types::ComponentItem) -> bool {
    use wasmtime::component::types::ComponentItem;
    match item {
        ComponentItem::ComponentFunc(_) | ComponentItem::CoreFunc(_) | ComponentItem::Module(_) | ComponentItem::Component(_) => true,
        ComponentItem::ComponentInstance(instance) => instance.exports(engine).any(|(_, ext)| is_callable(engine, &ext.ty)),
        ComponentItem::Type(_) | ComponentItem::Resource(_) => false,
    }
}

/// `Store<T>` data: nothing but the resource limiter. `DashboardContext`
/// never crosses into it — every kind this dashboard is allowed to see is
/// fetched host-side, in plain Rust, *before* the store even exists (see
/// `call`, below), so there's no borrowed, non-`'static` state that would
/// otherwise need bridging into wasmtime's `T: 'static` requirement.
struct HostState {
    limits: StoreLimits,
}

/// Runs one dashboard's `lines()` export: fetches every declared kind
/// host-side (plain `DashboardContext` calls, the same door `declarative`
/// uses), hands the results to the guest as data, and turns a trap (fuel
/// exhaustion, the memory limit, or a guest panic surfacing as a trap) into
/// a rendered line instead of propagating a crash or hanging the frame.
pub fn call(engine: &Engine, module: &WasmDashboardModule, kinds: &[(String, String)], ctx: &mut DashboardContext) -> Vec<UiLine<'static>> {
    let data: Vec<KindData> = kinds.iter().map(|(group, kind)| KindData { group: group.clone(), kind: kind.clone(), objects: ctx.fetch(group, kind).iter().filter_map(|v| serde_json::to_string(v).ok()).collect() }).collect();
    match run(engine, module, &data) {
        Ok(lines) => lines.into_iter().map(render_line).collect(),
        Err(e) => vec![UiLine::styled(format!("extension error: {e}"), Style::default().fg(theme().bad))],
    }
}

fn run(engine: &Engine, module: &WasmDashboardModule, data: &[KindData]) -> wasmtime::Result<Vec<Line>> {
    let limits = StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT_BYTES).instances(1).tables(4).memories(4).build();
    let mut store = Store::new(engine, HostState { limits });
    store.limiter(|state| &mut state.limits);
    store.set_fuel(FUEL_BUDGET)?;

    let linker = Linker::new(engine);
    let bindings = Dashboard::instantiate(&mut store, &module.component, &linker)?;
    bindings.call_lines(&mut store, data)
}

fn render_line(line: Line) -> UiLine<'static> {
    UiLine::from(line.spans.into_iter().map(render_span).collect::<Vec<_>>())
}

/// `style-hint` is a closed, semantic palette, not a raw color — this is the
/// one place that decides what each label actually looks like, same split
/// `declarative::render_widget` already draws for TOML widgets.
fn render_span(span: Span) -> UiSpan<'static> {
    let style = match span.style {
        StyleHint::Plain => Style::default(),
        StyleHint::Muted => Style::default().fg(theme().muted),
        StyleHint::Ok => Style::default().fg(theme().ok),
        StyleHint::Warn => Style::default().fg(theme().warn),
        StyleHint::Bad => Style::default().fg(theme().bad),
        StyleHint::Accent => Style::default().fg(theme().accent),
        StyleHint::Bold => Style::default().add_modifier(Modifier::BOLD),
        StyleHint::Heading => Style::default().fg(theme().heading).add_modifier(Modifier::BOLD),
    };
    UiSpan::styled(span.text, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real compiled component (see `examples/wasm-dashboard-demo/`, built
    /// with `cargo build --release --target wasm32-unknown-unknown` then
    /// `wasm-tools component new`), not a stand-in — this is what proves the
    /// whole compile/instantiate/call/render path actually works, not just
    /// that the Rust types line up.
    const DEMO: &[u8] = include_bytes!("testdata/demo.wasm");
    /// Same build, except `lines()` is `loop {}` — proves the fuel budget
    /// actually stops a runaway guest instead of hanging the caller.
    const BUSY: &[u8] = include_bytes!("testdata/busy.wasm");

    fn kind_data(objects: usize) -> KindData {
        KindData { group: "widgets.example.com".into(), kind: "Widget".into(), objects: (0..objects).map(|i| format!("{{\"metadata\":{{\"name\":\"w{i}\"}}}}")).collect() }
    }

    #[test]
    fn a_real_component_compiles_runs_and_renders_what_it_was_handed() {
        let engine = engine().unwrap();
        let module = WasmDashboardModule::compile_bytes(&engine, DEMO, "demo").unwrap();
        let lines = run(&engine, &module, &[kind_data(3)]).expect("a well-behaved component should run cleanly");
        let text: String = lines.iter().flat_map(|l| l.spans.iter()).map(|s| s.text.as_str()).collect();
        assert!(text.contains("3 objects seen"), "{text}");
    }

    #[test]
    fn a_runaway_guest_is_stopped_by_the_fuel_budget_not_hung_forever() {
        let engine = engine().unwrap();
        let module = WasmDashboardModule::compile_bytes(&engine, BUSY, "busy").unwrap();
        let result = run(&engine, &module, &[kind_data(1)]);
        assert!(result.is_err(), "a loop {{}} guest should trap on fuel exhaustion, not return");
    }

    #[test]
    fn a_runaway_guest_is_rendered_as_an_error_line_not_propagated() {
        let engine = engine().unwrap();
        let module = WasmDashboardModule::compile_bytes(&engine, BUSY, "busy").unwrap();
        let data: Vec<KindData> = vec![kind_data(1)];
        let lines = match run(&engine, &module, &data) {
            Ok(lines) => lines.into_iter().map(render_line).collect(),
            Err(e) => vec![UiLine::styled(format!("extension error: {e}"), Style::default().fg(theme().bad))],
        };
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("extension error:"), "{text}");
    }

    #[test]
    fn compiling_garbage_bytes_is_an_error_not_a_panic() {
        let engine = engine().unwrap();
        assert!(WasmDashboardModule::compile_bytes(&engine, b"not wasm", "garbage").is_err());
    }
}
