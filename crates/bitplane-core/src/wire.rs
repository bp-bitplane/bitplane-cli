//! What crosses a transport.
//!
//! The traits are the ergonomic face; this pair plus [`dispatch`] is what a
//! remote surface speaks (ADR-0003). `dispatch` is a `match`, so an action put
//! on the wire and forgotten in the dispatch fails to compile.
//!
//! One request is one user intent — coarse granularity, so a remote transport
//! is not a latency disaster — and a response is a single value, with streamed
//! progress deferred.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::engine::{Engine, Reader};
use crate::error::{EngineError, Problem};
use crate::member::{PROJECT_SIGIL, ProjectName, WorktreePath};
use crate::outcome::{Outcome, PerMember, PerProject};
use crate::plane_id::PlaneId;
use crate::project_file::ProjectSource;

/// One user intent.
///
/// Deliberately **not** `#[non_exhaustive]`: every surface is supposed to match
/// it exhaustively, so that an action added here and forgotten at a rendering
/// fails to compile — the same property [`dispatch`] has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Request {
    /// Make a plane, and a worktree of every member named.
    PlaneCreate(PlaneCreateRequest),
    /// Register a project from a URL, building the source repo bitplane owns.
    ProjectAdd(ProjectAddRequest),
    /// Every project registered on this host.
    ProjectList,
    /// Bring owned projects' source repos up to date with their forges.
    ProjectFetch(ProjectFetchRequest),
}

/// One answer. Never a scalar count: a fan-out answers with a vector of keyed
/// per-item outcomes, so a progress stream is later that vector arriving early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Response {
    PlaneCreate(PlaneCreated),
    ProjectAdd(ProjectAdded),
    ProjectList(ProjectListing),
    ProjectFetch(ProjectFetched),
}

/// Make a plane.
///
/// Members arrive as the strings the user wrote, because telling
/// `~/projects/weird:name` from `~/projects/bitplane:feat-x` needs the
/// filesystem, and the engine is the side that has one — the same reason
/// ADR-0003 puts plane discovery there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneCreateRequest {
    /// One or more members, as written: `@name`, or a path, each optionally
    /// carrying `:branch`.
    pub members: Vec<String>,
    /// The branch for every member that does not carry its own. Applied at
    /// this moment and not remembered: a plane has no branch.
    pub branch: Option<String>,
    /// The plane id. `None` generates one.
    pub id: Option<String>,
    pub intent: BranchIntent,
}

/// Whether the branch asked for must exist, must not, or either.
///
/// An intent flag rather than a lookup round trip: the ambiguity costs one
/// field instead of a "does this branch exist?" call (ADR-0003).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BranchIntent {
    /// Check the branch out if it resolves, and create it otherwise.
    #[default]
    Resolve,
    /// The branch must already exist.
    RequireExisting,
    /// The branch must not already exist.
    RequireNew,
}

/// A plane that was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneCreated {
    pub id: PlaneId,
    pub directory: PathBuf,
    /// One row per member, in the order they were named.
    pub members: Vec<PerMember<CreatedMember>>,
    /// Whether Ctrl-C stopped the run. The rows are then what was unwound and
    /// what was never attempted, and they are the repair instruction rather
    /// than a courtesy (ADR-0004).
    pub interrupted: bool,
    /// Whether the abort path could not take the whole plane back. Only an
    /// interrupted run can answer `Ok` with this set — a failed one says the
    /// same thing through `CreateAborted`'s own `remnant`, and a plane that was
    /// built has nothing to leave behind.
    pub remnant: bool,
}

/// One member's worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedMember {
    /// The branch the worktree is on.
    pub branch: String,
    /// Whether that branch had to be cut. The difference between joining a
    /// colleague's work and starting your own.
    pub created_branch: bool,
    /// Where the worktree is, relative to the plane directory.
    pub path: WorktreePath,
    /// Whether the abort path took it back again.
    pub unwound: bool,
}

/// Register a project from a URL.
///
/// One request per intent, not one per command line: a default name colliding
/// and being re-run with `--name <suggestion>` is **two** requests, because the
/// caller supplied new information in between (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAddRequest {
    /// Anything git can fetch from. bitplane speaks the git protocol and no
    /// forge API, and never reads, stores, prompts for or forwards a
    /// credential.
    pub url: String,
    /// The project's name. `None` takes the last segment of the URL's path.
    pub name: Option<String>,
}

/// Bring owned projects up to date.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFetchRequest {
    /// Which projects, as `@name` or `name`. **Empty takes every registered
    /// project in turn.**
    pub projects: Vec<String>,
}

/// A project that was registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAdded {
    pub name: ProjectName,
    pub source: ProjectSource,
    /// The project directory: `project.toml`, `repo.git`, and the `bin/` the
    /// user may create.
    pub directory: PathBuf,
    /// The base a new plane branch would be cut from. `None` where the forge
    /// named no default — not an error by itself, and only refused at the
    /// moment a new branch has to be cut (ADR-0005).
    pub default_branch: Option<String>,
}

/// Every project on this host, one row each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectListing {
    pub projects: Vec<PerProject<ProjectSummary>>,
}

/// One project, as a listing sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub source: ProjectSource,
    /// Whether the source repo is on this host. One `stat`, never a stored
    /// field (ADR-0007).
    pub source_repo_present: bool,
}

/// What a fetch did, project by project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFetched {
    pub projects: Vec<PerProject<Fetched>>,
    /// Whether Ctrl-C stopped the run. The remaining rows are then `skipped`.
    pub interrupted: bool,
}

/// One project's fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    /// How many `refs/remotes/origin/*` this fetch added, moved or pruned.
    /// Zero is "up to date".
    pub updated: usize,
}

impl ProjectListing {
    /// Whether any project's file could not be read.
    ///
    /// A listing tolerates a file it cannot read — the row is the result and
    /// the scan carries on — so this is what turns it into drift rather than a
    /// failure (ADR-0003).
    pub fn has_unreadable(&self) -> bool {
        self.projects.iter().any(PerProject::is_failure)
    }
}

impl ProjectFetched {
    /// The failure a run with at least one failed fetch reports **alongside**
    /// its rows.
    ///
    /// The rows are the result and go to stdout regardless; this is what says
    /// the run did not fully succeed. `None` where every fetchable project was
    /// fetched.
    pub fn failure(&self) -> Option<EngineError> {
        let problems: Vec<Problem> = self
            .projects
            .iter()
            .filter_map(|row| match &row.outcome {
                Outcome::Failed(error) => Some(Problem::about(
                    format!("{PROJECT_SIGIL}{}", row.project),
                    error.to_string(),
                )),
                _ => None,
            })
            .collect();

        if problems.is_empty() {
            return None;
        }

        Some(EngineError::FetchFailed {
            failed: problems.len(),
            fetchable: self.fetchable(),
            problems,
        })
    }

    /// The projects there was something to fetch **for**. An adopted project is
    /// a row, never a denominator.
    fn fetchable(&self) -> usize {
        self.projects
            .iter()
            .filter(|row| {
                !matches!(
                    row.outcome,
                    Outcome::Skipped(crate::outcome::SkipReason::NothingToFetch)
                )
            })
            .count()
    }
}

/// Runs one request against one engine.
pub fn dispatch<E: Engine + ?Sized>(engine: &E, request: Request) -> Result<Response, EngineError> {
    match request {
        Request::PlaneCreate(request) => engine.plane_create(request).map(Response::PlaneCreate),
        Request::ProjectAdd(request) => engine.project_add(request).map(Response::ProjectAdd),
        Request::ProjectList => Reader::project_list(engine).map(Response::ProjectList),
        Request::ProjectFetch(request) => engine.project_fetch(request).map(Response::ProjectFetch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_over_the_wire() {
        let request = Request::PlaneCreate(PlaneCreateRequest {
            members: vec!["@api".to_owned(), "~/projects/bitplane:feat-x".to_owned()],
            branch: Some("feat-login".to_owned()),
            id: Some("auth-work".to_owned()),
            intent: BranchIntent::RequireNew,
        });

        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains(r#""action":"plane_create""#), "got {json}");
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);
    }

    #[test]
    fn the_default_intent_is_to_resolve() {
        assert_eq!(BranchIntent::default(), BranchIntent::Resolve);
    }
}
