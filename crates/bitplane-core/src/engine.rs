//! The two traits every mutation and every read passes through (ADR-0003).
//!
//! The split into [`Reader`] and [`Engine`] exists so ADR-0002's *"a read never
//! writes"* is checkable by reading the trait rather than by auditing every
//! body. In Rust both reads and mutations take `&self` — the state is the
//! filesystem — so `&self`/`&mut self` cannot express the distinction.
//!
//! Sync throughout, per ADR-0001: the whole workload is spawn-subprocess-and-wait
//! plus small TOML reads and writes, so there is no IO multiplexing problem and
//! async would colour every caller for nothing.
//!
//! There is no `host` parameter. The host is chosen by picking which engine you
//! hold — [`crate::LocalEngine`] now, an SSH engine later.
//!
//! Actions arrive one slice at a time, each landing on the trait, on
//! [`crate::Request`] and in [`crate::dispatch`] together.

use crate::error::EngineError;
use crate::wire::{
    PlaneCreateRequest, PlaneCreated, ProjectAddRequest, ProjectAdded, ProjectFetchRequest,
    ProjectFetched, ProjectListing,
};

/// The reads: `plane_list`, `plane_show`, `plane_status`, `project_list`,
/// `project_show` and `doctor`. None of them takes a lock or creates a file.
pub trait Reader {
    /// Every project registered on this host.
    ///
    /// One `readdir` and one small file per project. A file that will not parse
    /// is a row in an error state and the scan continues, because a listing
    /// that dies on one bad file says nothing about the other nine.
    fn project_list(&self) -> Result<ProjectListing, EngineError>;
}

/// The reads, plus every mutation.
pub trait Engine: Reader {
    /// Makes a plane and a worktree of every member named.
    ///
    /// Strict, not idempotent: it either makes a whole plane or it makes none.
    /// A failure returns `Err` carrying the per-member rows, because the
    /// envelope `Err` is for an operation that produced no durable state.
    fn plane_create(&self, request: PlaneCreateRequest) -> Result<PlaneCreated, EngineError>;

    /// Registers a project from a URL, building the source repo bitplane owns.
    ///
    /// A failure keeps the object store and unwinds only the registration: the
    /// cold fetch is the expensive step, and `init --bare` + `fetch` is
    /// resumable in a way `clone` is not (ADR-0005).
    fn project_add(&self, request: ProjectAddRequest) -> Result<ProjectAdded, EngineError>;

    /// Brings owned projects' source repos up to date with their forges.
    ///
    /// A fan-out: a forge that will not answer is one row, and every other
    /// project still gets its turn.
    fn project_fetch(&self, request: ProjectFetchRequest) -> Result<ProjectFetched, EngineError>;
}
