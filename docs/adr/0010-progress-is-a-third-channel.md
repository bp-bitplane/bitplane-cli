# ADR-0010: Progress is a third channel: an fd, not stdout or stderr

Status: accepted, amends [ADR-0003](./0003-the-engine-contract.md)
Date: 2026-11-12
Ticket: bp-bitplane/bitplane-cli#44 (spec bp-bitplane/bitplane-protocol#8)

## Context

A daemon is about to drive `bp` on a user's behalf: list planes, take them
apart, and — the case that matters here — create them while a window shows
which member is fetching, which worktree is being added and which script is
running. It needs two things the CLI does not offer.

**A way in that is not argv.** The daemon holds a `Request` already; building a
command line out of it means a second renderer of the wire, kept in step with
clap by hand, in another language. `bp` already has the choke point that makes
this unnecessary: every command line becomes a `Request` and goes through
`dispatch()`. The argv parser is a front end to that, not the only one.

**Progress it can read.** [ADR-0003](./0003-the-engine-contract.md) deferred
streamed progress and named its cost: a `create` of six members sits silent on
the slowest clone. It also left the hedge in place — every fan-out answers with
rows keyed by a stable id, so *a progress event is a partial row arriving
early*, and the sink belongs on the Engine rather than on each call.

What constrains the channel is what the other two already mean, and neither can
move:

- **stdout is one result.** `bp create --json | jq` gets exactly one value, and
  a pipeline that receives an event where it expected a plane is a broken
  contract.
- **stderr is the human stream and the envelope.** Lock waits, script output
  and the failure all go there, in both renderings, so a terminal shows what is
  happening and the last line is machine-readable. Interleaving JSON events with
  `pnpm`'s banner would make neither readable.

## Decision

### A `Request` on stdin, through `dispatch()`

`bp --stdin` reads **one** `Request` as JSON from stdin, runs `dispatch` on it,
and from there is indistinguishable from the argv path: the `Response` on
stdout, the `ErrorEnvelope` on stderr, the same exit codes — `3` for findings
included. The global flags still apply, so `--json` chooses the rendering of
both, human by default, and `--planes-dir` the directories. `--stdin` does not
imply `--json`: flags that combine freely are easier to reason about than one
that switches another on. A subcommand together with `--stdin` is a usage
error; input that is not a `Request` is `malformed_request`, a usage error, exit
`2`.

No argv renderer is written on the other side. Anything a command line can ask
for, a `Request` can, because a command line *becomes* one.

`bp --version --json` prints `{"version": "<semver>", "contract": <n>}`. The
contract number is the wire's version, bumped when a `Request`, `Response`,
envelope or progress event changes in a way an existing client would misread.
The binary's version says which `bp`; the contract says whether a client built
against an older one can talk to it.

### Progress on a caller-chosen fd, as JSON lines

`--progress-fd <n>` writes progress events to fd `n`, one JSON object per line.
The caller opens the fd — a pipe, a file, `3>progress.jsonl` — and `bp` only
writes to it. stdout and stderr are **byte-identical** with and without it.

An event is a partial row keyed by `MemberRef`, or a lock wait:

```json
{"event":"member","member":"@api","step":"fetch","state":"started"}
{"event":"member","member":"@api","step":"worktree","state":"done"}
{"event":"member","member":"@api","step":"script","state":"failed","name":"install","log":"/…/install.log"}
{"event":"waiting","object":"/…/planes/auth-work"}
```

`step` is `fetch`, `worktree` or `script`; `state` is `started`, `done` or
`failed`. Script events carry the script's `name` and its `log`, so a watcher
can tail the file the result will name. `create` is the only emitter today; the
flag is global because under `--stdin` the command line does not say which
action is coming.

The sink is not where ADR-0003 predicted it. ADR-0003 sketched
`Engine::set_progress`, a default method on the trait; the sink is
`LocalEngine::reporting_progress` instead, next to `announcing_lock_waits`.
`LocalEngine` is the only engine, and a trait method with one implementation
would guess at what a second engine needs. The trait gains one when a second
engine exists. What ADR-0003 relied on still holds: the sink is a callback on
the worker thread, with no channel and no runtime. `ProgressEvent` lives beside
the wire types.

### No terminal event

The stream ends at EOF, and what it amounted to is the exit status and the
result on stdout. A terminal `done` event would be a second answer to *how did
it go*, and a second answer is one that can disagree with the first — a reader
that trusts the event over the exit code is wrong exactly when it matters. EOF
also arrives when `bp` is killed, which a terminal event never would.

That only holds if `bp` is the last writer, so **the fd is close-on-exec**: a
script that daemonises a watcher would otherwise inherit it and hold the stream
open long after `bp` exited.

### Write errors are ignored

A reader that goes away — a closed pipe, a full disk — loses its progress and
nothing else. `bp` finishes the operation, writes its result and exits as it
would have. Progress is advisory; aborting a half-built plane because nobody is
watching any more would turn an observer into a participant.

The opposite mistake is caught up front: an fd that is not open, or is one of
`0`, `1` and `2`, is `bad_progress_fd`, a usage error, exit `2`, before anything
is fetched or created.

## Consequences

- The daemon spawns `bp --stdin --json --progress-fd 3` with a `Request` on
  stdin and needs no `bitplane-core` and no argv builder.
- Events from the parallel worktree fan-out interleave across members. Each line
  is written whole, so a line never tears; order is only meaningful within one
  member.
- A reader cannot tell "no events" from "nothing to report yet". It does not
  need to: EOF is the end, and a step that never started has no row.
- `fetch` events exist only for owned projects; an adopted project or an ad-hoc
  member has nothing to fetch and goes straight to `worktree`.

## Alternatives considered

**Events on stderr, tagged.** Rejected. stderr is a human stream with script
output interleaved at byte granularity; a reader would have to find JSON lines
inside `pnpm`'s output, and a script printing something JSON-shaped would forge
an event.

**Events on stdout, followed by the result.** Rejected: it breaks
`bp create --json | jq`, which every existing caller relies on, and makes the
result the last of N values rather than the only one.

**A terminal `result` event carrying the response.** Rejected as a second copy
of stdout; see *No terminal event*.

**Failing the run when the fd cannot be written.** Rejected; see *Write errors
are ignored*.

**A request file argument instead of stdin.** Rejected: it needs a temporary
file, and stdin is the one input every spawner already has.
