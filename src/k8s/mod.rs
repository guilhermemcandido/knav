//! Everything that talks to the Kubernetes API or shapes what it returns
//! into rows: one module per concern, re-exported flat so callers keep
//! writing `k8s::Thing`.

pub mod catalog;
pub mod describe;
pub mod details;
pub mod layout;
pub mod relations;
pub mod report;
pub mod metrics;
pub mod scope;
pub mod sort;
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
mod instances;
mod parallel;
mod watch;
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
pub use instances::{Count, Counter, InstanceCounts, count_key};
pub use parallel::par_map;
pub use watch::{changes, watch_count, watch_store};
pub use watched::*;
