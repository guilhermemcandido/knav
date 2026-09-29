//! Everything that talks to the Kubernetes API or shapes what it returns
//! into rows: one module per concern, re-exported flat so callers keep
//! writing `k8s::Thing`.

pub mod catalog;
pub mod describe;
pub mod details;
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
mod helm;
mod instances;
mod parallel;
mod watch;
mod watched;
mod table;
mod kept;

pub use kind::*;
pub use pods::*;
pub use context::*;
pub use age::*;
pub use apis::*;
pub use table::*;
pub use deployments::*;
pub use events::*;
pub use nodes::*;
pub use overview::*;
pub use generic::*;
pub use helm::HelmStore;
pub use instances::{Count, Counter, InstanceCounts, count_key};
pub use parallel::par_map;
pub use watch::{Feed, changes, note_change, watch_count, watch_live, watch_store, watch_store_selected};
pub use kept::{AgeRow, Item, Kept};
pub type PodKept = Kept<k8s_openapi::api::core::v1::Pod, PodRow>;
pub type DeploymentKept = Kept<k8s_openapi::api::apps::v1::Deployment, DeploymentRow>;
pub use watched::*;
