//! What knav does to the cluster and the machine: actions, edits, shells, port-forwards,
//! your own commands, the clipboard and saved logs.

pub mod actions;
pub mod clipboard;
pub mod custom;
pub mod edit;
pub mod logfile;
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
