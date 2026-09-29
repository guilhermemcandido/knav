//! WASM dashboards. A guest gets `wit/dashboard.wit`: one export and no callable
//! imports. `run` registers nothing on its `Linker`, so compiled code can't reach
//! a client, a file or the network.

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

/// Budget for one `lines()` call. Fuel caps instructions, so a runaway guest can't
/// hang the render loop; the memory limit is separate, since fuel ignores allocation.
const FUEL_BUDGET: u64 = 200_000_000;
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// One engine for the process. Fuel metering is an engine setting, so every store
/// built from it is bounded.
pub fn engine() -> Result<Engine, String> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    Engine::new(&config).map_err(|e| e.to_string())
}

/// One extension's compiled dashboard. Compiled once at load, run every frame it shows.
#[derive(Clone)]
pub struct WasmDashboardModule {
    component: Component,
}

impl WasmDashboardModule {
    /// Compiles `path`, already checked by `resolve_wasm_dashboard`. A bad file is an
    /// `Err`, so it never stops the other extensions loading.
    pub fn compile(engine: &Engine, path: &std::path::Path) -> Result<Self, String> {
        let component = Component::from_file(engine, path).map_err(|e| format!("compiling {}: {e}", path.display()))?;
        Self::from_component(engine, component, &path.display().to_string())
    }

    /// `compile` for a fixture already in memory.
    #[cfg(test)]
    fn compile_bytes(engine: &Engine, bytes: &[u8], label: &str) -> Result<Self, String> {
        let component = Component::new(engine, bytes).map_err(|e| format!("compiling {label}: {e}"))?;
        Self::from_component(engine, component, label)
    }

    /// Refuses any import a guest could call: a function, module or component. Type
    /// imports are allowed, since the component format uses them to share types and
    /// they carry no capability.
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

/// Store data: just the limiter. Every kind is fetched before the store exists,
/// so nothing borrowed crosses into the guest.
struct HostState {
    limits: StoreLimits,
}

/// Runs a dashboard: fetches its declared kinds host-side, hands them to the guest
/// as data, and turns a trap (fuel, memory, panic) into an error line.
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

/// The guest picks a meaning from a closed set; the colour is decided here, so a
/// dashboard can't style itself to look like another part of knav.
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

    /// A real compiled component, built from `examples/wasm-dashboard-demo/`.
    const DEMO: &[u8] = include_bytes!("testdata/demo.wasm");
    /// Same, but `lines()` loops forever, to check the fuel budget stops it.
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
