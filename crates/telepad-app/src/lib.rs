//! Telepad application layer, Model–View–Update style.
//!
//! - [`State`] = [`Model`] (changed only by [`update`]) + [`Drafts`] (form
//!   text, filter and toggles that views bind to directly).
//! - Views read the model and push [`Msg`]s.
//! - [`update`] applies one message and returns [`Effect`]s; it never does
//!   I/O and gets the time as a parameter, so it is fully unit-testable.
//! - The GUI runtime executes effects and feeds their results back as messages.
//!
//! This crate does not depend on any GUI toolkit.

mod msg;
mod state;
mod table;
mod update;

pub use msg::{Effect, Msg};
pub use state::{
    Drafts, Editor, HostState, ImportDialog, MESSAGE_TTL, Model, OverviewPage, RunInfoView, State,
    UpdatePhase, UpdateState, matches_filter,
};
pub use table::{
    Flag, FlagKind, HostGroup, Severity, Sort, SortKey, StackRow, cluster_shown_by, docker_stacks,
    fullest_disk, host_flags, host_list_groups, host_urgency, hottest, maintenance_flags,
    overview_rows, urgency, worst_usage,
};
pub use update::update;

#[cfg(test)]
mod tests;
