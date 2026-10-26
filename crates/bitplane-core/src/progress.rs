//! Where a running fan-out reports what it is doing (ADR-0010).
//!
//! A sink rather than a channel or an iterator, for the reason the lock
//! announcement and the script stream are closures: the engine owns no
//! terminal and no file descriptor, and a surface that wants the events says
//! so. Without one, nothing is reported.

use crate::wire::ProgressEvent;

/// Called once per event, from whichever worker thread the step ran on.
pub type ProgressSink = dyn Fn(&ProgressEvent) + Sync;

/// The sink of every action that reports no progress, and of a caller that
/// asked for none.
pub fn silent(_: &ProgressEvent) {}
