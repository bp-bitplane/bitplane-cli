---
title: bp request
---

# `bp request`

:::implemented

Everything on this page works as described.

:::

Run one request, read as JSON from stdin, instead of building it from a command
line.

```
bp request [--json] [--progress-fd <fd>] < request.json
```

`request` is for **programs** that drive `bp`: a daemon, an editor, a script
that already holds the structured value. It skips the argument parser and hands
the request straight to the same dispatch every other command passes through,
so what comes back is exactly what the equivalent command line would have
printed.

## Input

stdin holds **one** request: a single JSON value whose `action` names what to
do, and whose other fields are that action's arguments. It is the serialisation
of the core `Request` type — the same value `bp` builds from a command line —
so this page does not repeat its fields. The generated TypeScript contract,
[`bp-contract.ts`](https://github.com/bp-bitplane/bitplane-cli/blob/main/crates/bitplane-core/bp-contract.ts),
is the place to read them.

```sh
$ echo '{"action":"plane_list","health":"cheap"}' | bp request --json
```

Paths in a request are taken **as given**. There is no current directory to
resolve them against, so a program sends absolute paths: a plane named by the
directory it contains is `{"plane":"containing_path","path":"/abs/path"}`, and
`bp create --here` is a `here` holding the directory itself.

stdin is read to its end before anything runs. Input that is not one well-formed
request — not JSON, an unknown `action`, a missing or mistyped field, or nothing
at all — is a usage failure, reported before git is checked or any directory is
touched:

```sh
$ echo '{"action":"plane_show"}' | bp request
```
```
error[malformed_request]: stdin is not a request: missing field `plane`

remedy: Send exactly one request, as JSON, on stdin.
```

## Arguments

None. The request carries everything.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `--progress-fd <fd>` | — | Write progress as JSON lines to this already-open file descriptor, exactly as [`bp create --progress-fd`](./plane/create.md#progress) does. Only a `plane_create` request reports any. |

Plus the [global flags](./global-flags.md#global-flags).

## Output

Exactly what the equivalent command line prints, on the same streams and with
the same [exit code](./exit-codes.md): the response on stdout, a failure as the
[error envelope](./refusals-and-waivers.md#the-error-envelope) on stderr, a
finding as exit `3` with the result still on stdout, and Ctrl-C as `130`.

[`--json`](./global-flags.md#--json) selects the rendering here as everywhere
else. A program almost always wants it.

```sh
$ echo '{"action":"plane_show","plane":{"plane":"id","id":"auth-work"},"health":"cheap"}' | bp request --json
```

## The contract version

`bp --version --json` reports which revision of the request and response types
this `bp` speaks, so a program can refuse a `bp` it was not built against
before it sends anything:

```sh
$ bp --version --json
{"name":"bp","version":"0.1.0","contract":1}
```

`contract` is a whole number. It goes up whenever a request or response changes
shape in a way an older client would misread, and only then.

## Exit codes

| code | when |
| --- | --- |
| `2` | stdin did not hold one well-formed request |
| any other | whatever the request's own command exits with |

## See also

- [Global flags](./global-flags.md) — `--json`, and the directories every request resolves
- [Exit codes](./exit-codes.md)
