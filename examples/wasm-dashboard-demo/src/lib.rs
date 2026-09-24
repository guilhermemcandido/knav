// A worked example proving the WASM dashboard path end to end: real Rust,
// compiled to a component, with zero imports (see ../../wit/dashboard.wit).
// It counts the objects it's handed and renders one line — enough to prove
// the whole round trip (manifest -> compile -> instantiate -> fetch ->
// call -> render) without needing real cluster data to be interesting.
//
// Build (from this directory):
//   rustup target add wasm32-unknown-unknown
//   cargo build --release --target wasm32-unknown-unknown
//   wasm-tools component new \
//     target/wasm32-unknown-unknown/release/demo_dashboard.wasm \
//     -o dashboard.wasm

wit_bindgen::generate!({
    path: "../../wit/dashboard.wit",
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
