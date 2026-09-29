// The demo WASM dashboard: counts the objects it's handed and renders one line.
// Build: `cargo build --release --target wasm32-unknown-unknown`, then
// `wasm-tools component new target/wasm32-unknown-unknown/release/demo_dashboard.wasm -o dashboard.wasm`.

wit_bindgen::generate!({
    path: "../../crates/knav-extensions/wit/dashboard.wit",
    world: "dashboard",
});

use crate::knav::extension::types::{Span, StyleHint};

struct Demo;

impl Guest for Demo {
    fn lines(data: Vec<KindData>) -> Vec<Line> {
        let total: usize = data.iter().map(|k| k.objects.len()).sum();
        vec![Line { spans: vec![Span { text: format!("{total} objects seen"), style: StyleHint::Accent }] }]
    }
}

export!(Demo);
