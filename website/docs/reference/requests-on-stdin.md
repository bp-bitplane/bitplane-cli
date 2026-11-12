---
title: Requests on stdin
---

# Requests on stdin

:::not-implemented

`bp --stdin` does not exist yet. This page is the specification it is being
built against, not a description of the binary.

:::

Drive `bp` with a request as JSON instead of a command line.

```
bp --stdin [--json] [--progress-fd <n>] < request.json
```

Every command line `bp` parses becomes a **request**, and the request is what
runs. `--stdin` skips the parser: `bp` reads one request from stdin and runs it.
Whatever a command line can ask for, a request can — a program that already
holds one needs no argv builder.

```sh
echo '{"action":"plane_list","health":"cheap"}' | bp --stdin --json
```

## What comes back

Exactly what the equivalent command line prints: the response on stdout, a
failure as the [error envelope](./refusals-and-waivers.md#the-error-envelope)
on stderr, and the same [exit codes](./exit-codes.md) — `3` for a finding
included. The [global flags](./global-flags.md#global-flags) still apply, so
`--json` picks the rendering.

| input | exit | stderr |
| --- | --- | --- |
| a request that works | `0` | — |
| a request that reports a finding | `3` | — |
| a request that is refused | `1` | the envelope |
| input that is not a request | `2` | `malformed_request` |
| `--stdin` with a subcommand | `2` | `bad_usage` |

## The request

One JSON object, tagged by `action`. The actions and their fields are the core
request types; [generated TypeScript](https://github.com/bp-bitplane/bitplane-cli/blob/main/crates/bitplane-core/bp-contract.ts)
is the copy to read and import.

```json
{"action":"plane_create","members":["@api","@web"],"branch":"feat-login","id":null,"here":null,"intent":"resolve","fetch":true,"run_scripts":true}
{"action":"plane_destroy","plane":{"plane":"id","id":"auth-work"},"waive":[],"run_scripts":true}
{"action":"project_list"}
```

Paths in a request are taken as written. A command line resolves `bp status`
against the current directory; a request says `{"plane":"containing_path",
"path":"/Users/alfonz/planes/auth-work"}` instead.

## The contract version

```sh
bp --version --json
```

```json
{"version":"0.1.0","contract":1}
```

`version` is the binary's. `contract` is the wire's, and it changes only when a
request, response, envelope or [progress event](./progress-fd.md) changes in a
way an existing client would misread. A client checks `contract`.

## See also

- [Progress on an fd](./progress-fd.md) — what `bp create` reports while it works
- [Global flags](./global-flags.md#--json) — how the response is rendered
