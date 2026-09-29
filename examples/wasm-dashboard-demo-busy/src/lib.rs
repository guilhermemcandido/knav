// A deliberately misbehaving dashboard component, used only as a test
// fixture (see src/extensions/dashboards/testdata/busy.wasm and
// dashboards::wasm's tests) to prove the host's fuel budget actually stops
// a runaway guest instead of hanging the render loop.
//
// Build the same way as ../wasm-dashboard-demo, see its src/lib.rs.

wit_bindgen::generate!({
    path: "../../crates/knav-extensions/wit/dashboard.wit",
    world: "dashboard",
});

struct Busy;

impl Guest for Busy {
    fn lines(_data: Vec<KindData>) -> Vec<Line> {
        loop {}
    }
}

export!(Busy);
