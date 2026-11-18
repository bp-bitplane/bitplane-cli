---
title: Progress on an fd
---

# Progress on an fd

:::implemented

Everything on this page works as described.

:::

Report what `bp create` is doing, member by member, as JSON lines on a file
descriptor you open.

```
bp create <member>... --progress-fd <n>
```

stdout keeps carrying one result and stderr keeps carrying the human stream and
the envelope. Progress is a **third channel**: the caller opens fd `n` — a pipe,
or a file — and `bp` writes one JSON object per line to it while it works. Both
other streams are byte-identical with and without the flag.

```sh
bp create @api @web -b feat-login --json --progress-fd 3 3>progress.jsonl | jq .id
```

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `--progress-fd <n>` | — | Write progress events to fd `n`, one JSON object per line. |

It is a [global flag](./global-flags.md#global-flags), so it works with a
[request on stdin](./requests-on-stdin.md) too. `bp create` is the only command
that reports anything on it today.

## The events

A **member event** is a partial row of the result, keyed by the member:

```json
{"event":"member","member":"@api","step":"fetch","state":"started"}
{"event":"member","member":"@api","step":"fetch","state":"done"}
{"event":"member","member":"@api","step":"worktree","state":"started"}
{"event":"member","member":"@api","step":"worktree","state":"done"}
{"event":"member","member":"@api","step":"script","state":"started","name":"install","log":"/Users/alfonz/planes/auth-work/.bitplane/logs/20261112T101500Z-api-install.log"}
{"event":"member","member":"@api","step":"script","state":"done","name":"install","log":"/Users/alfonz/planes/auth-work/.bitplane/logs/20261112T101500Z-api-install.log"}
```

| field | values |
| --- | --- |
| `member` | `@name` for a project, the absolute repo path for an ad-hoc member |
| `step` | `fetch`, `worktree` or `script` |
| `state` | `started`, `done` or `failed` |
| `name`, `log` | the script and the log its output is tee'd to; `script` steps only |

A **waiting event** says a lock is held by another `bp`, the same moment the
human stream says `waiting for …`:

```json
{"event":"waiting","object":"/Users/alfonz/planes/auth-work"}
```

Only owned projects are fetched, so an adopted project or an ad-hoc member
starts at `worktree`. Worktrees are built in parallel: lines for different
members interleave, and each line is written whole.

## How the stream ends

**There is no final event.** The stream ends at EOF, and what the run amounted
to is the exit code and the result on stdout — the same answer a caller without
the flag gets. EOF arrives however `bp` ends, killed included. The fd is not
inherited by scripts, so a script that leaves a process behind cannot hold the
stream open.

## When the fd is wrong

An fd that is not open, is open only for reading, or is `0`, `1` or `2`, fails
before anything is fetched or created. Exit `2`, `bad_progress_fd`.

A reader that goes away halfway costs only the progress. `bp` ignores a failed
write, finishes the plane, and exits as it would have.

## See also

- [`bp create`](./plane/create.md) — the command that reports progress
- [Requests on stdin](./requests-on-stdin.md) — the other input a program uses
