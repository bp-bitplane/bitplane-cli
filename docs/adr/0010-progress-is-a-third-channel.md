# ADR-0010: Progress is a third channel: an fd, not stdout or stderr

Status: accepted, pays off the progress hedge in [ADR-0003](./0003-the-engine-contract.md) for `create`
Date: 2026-10-26
Ticket: [bp create reports structured progress](https://github.com/bp-bitplane/bitplane-cli/issues/37), built in [#39](https://github.com/bp-bitplane/bitplane-cli/issues/39)

## Context

bitplane-sessiond runs `plane.create` as a job. A job has a `phase` —
fetching, adding worktrees, running scripts — and a final status, and the
daemon has to learn both from `bp`, because it runs `bp` as a child process and
never links `bitplane-core`.

`bp` has two streams and both are spoken for. **stdout carries results only**:
one value, so `bp create --json | jq` receives exactly one thing (ADR-0003).
**stderr is the human stream**: the envelope, `waiting for …`, and every
script's merged output, verbatim (ADR-0007). Progress fits in neither without
breaking what the other readers of that stream rely on.

ADR-0003 named the cost of a response that is a single value: a parallel fan-out
shows nothing until its slowest member finishes. It deferred streamed progress
rather than ruling it out, and shaped every fan-out as a vector of keyed rows so
a progress stream could later be *that vector arriving early*.

## Decision

**`--progress-fd <n>` names a third channel**, an already-open file descriptor
that `bp` writes JSON lines to. `bp create` takes it, and so does `bp request`,
which is how the daemon runs a create.

- **stdout and stderr are byte-identical with and without it.** Log lines stay
  on stderr; the daemon's job log is stderr verbatim, and the events' script
  markers are what attribute a stretch of it to a script.
- **Events are partial rows keyed by `MemberRef`**, the same key every plane
  fan-out uses:
  `{"event":"member","member":"@api","step":"fetch"|"worktree"|"script","state":"started"|"done"|"failed"}`,
  where a script event also carries its `name` and `log`. A contended lock is
  `{"event":"waiting","object":"<path>"}`.
- **There is no terminal event.** EOF and the exit status end the stream; the
  final status comes from `bp`'s result, never from the disk and never from the
  last event.
- **The channel is never load-bearing.** A write that fails is dropped and `bp`
  carries on. Cancelling is SIGINT, as it is for a person.
- **A descriptor that is not open for writing is a usage failure**, exit `2`,
  before anything is resolved, fetched or created. So are `0`, `1` and `2`:
  pointing progress at stdout or stderr would break the property above.
- **The sink is on the Engine**, not in the CLI: `LocalEngine::reporting_progress`
  takes a closure, the same seam as `announcing_lock_waits`. `create` is the only
  emitter; every other action ignores it. `ProgressEvent` is a wire type, beside
  `Request` and `Response`.

## Considered options

- **JSON lines on stdout, the result last.** Rejected: it changes what `--json`
  stdout means for one command, so `bp create --json | jq` stops receiving one
  value, and it reopens ADR-0003 to buy nothing the fd does not.
- **Tagged lines on stderr.** Rejected: any script that prints the tag breaks
  the framing — the exact contradiction ADR-0003's amendment removed when it
  made stderr the human stream.
- **Phase transitions only.** Rejected: a coarser copy of the per-member rows,
  in a vocabulary the next fan-out could not reuse.

## Consequences

- The fd works only when whoever spawns `bp` holds the other end of a pipe.
  That is always true for the daemon, which runs `bp` on its own host; a remote
  host runs its own daemon behind the gateway.
- ADR-0003's named cost is paid off for `create` only. `add`, `destroy` and
  `project fetch` can adopt the same events later without a contract change,
  because the event is keyed the way their rows already are.
- A consumer derives a phase as the furthest step any member has started, and
  must tolerate an event it does not recognise: new steps are additive.
