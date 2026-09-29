// A dashboard that loops forever, the test fixture proving the fuel budget stops a
// runaway guest. Built like ../wasm-dashboard-demo.

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
