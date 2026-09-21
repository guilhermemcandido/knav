//! Everything that talks to the Kubernetes API or shapes what it returns
//! into rows: one module per concern, re-exported flat so callers keep
//! writing `k8s::Thing`.

mod kind;
mod pods;
mod context;
mod age;
mod apis;
mod deployments;
mod events;
mod nodes;
mod overview;
mod generic;
mod watched;

pub use kind::*;
pub use pods::*;
pub use context::*;
pub use age::*;
pub use apis::*;
pub use deployments::*;
pub use events::*;
pub use nodes::*;
pub use overview::*;
pub use generic::*;
pub use watched::*;
