//! What knav does to the cluster and the machine: actions, editing, shells, port-forwards, the clipboard.

pub mod actions;
pub mod clipboard;
pub mod edit;
pub mod portforward;
pub mod shell;

/// What an operation did, for the notice shown afterwards.
pub struct Outcome {
    pub text: String,
    pub tone: NoticeTone,
}

/// How a notice reads: finished, just information, or failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Done,
    Info,
    Failed,
}
