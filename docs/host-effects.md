# Native Host effects

English | [日本語](host-effects.ja.md)

The optional `host-effects` feature adds an explicitly invoked native Host. The ordinary `sayo` CLI, Wasm, and
Playground evaluator remain pure. The Host registers permitted operations under handles and processes Requests left
in the whole NormalForm.

## Run

```sh
cargo run --locked --features host-effects --bin sayo-host -- \
  --config examples/host_effects/host.json examples/host_effects/main.sayo
```

This example actually reads the clock, obtains 16 bytes from the OS random source, and writes a notification to stderr.
stdout contains three `Succeeded{Data<blob:...>}` responses; stderr contains the notification and operation counts.
Linux builds require OpenSSL development libraries and pkg-config.

Add `--export-blobs /tmp/sayo-observations` to save each observed result as `ID.blob`. Existing files are never
overwritten. Random bytes and HTTP bodies need not be UTF-8. Export is explicit because results can contain external data.
An operation's `Failed` response is ordinary program input and does not by itself set a failing CLI exit code.
Configuration errors, Host limits, and failed pure evaluations exit with a nonzero status.

## Envelopes

```sayo
Request{Service<handle:clock>, Data<blob:empty>}
```

`Service` refers to a registered operation; `Data` refers to an immutable input blob in the Host. Responses are
`Succeeded{Data<blob:new-id>}` or `Failed{Data<blob:new-id>}`. Using the same `Data` role for input and output allows a
returned Term to enter the next Request. Sayo never interprets blob contents.

Without `--world`, the CLI uses the ordinary user World from `effect_world()`. It declares Service/Data checkers,
captures `send Service<handle:...> Data<blob:...>` into Request, provides success/failure captures, and uses
`template.sayo` to validate payload Spaces. These are not new Native Relations. To supply your own World, pass
`--world PATH` and declare the same envelopes. Reference kinds and registrations are validated by the Host.
Host writes must not modify the World or referenced source files.

## Operations

| `kind` | Input blob | Successful output blob |
| --- | --- | --- |
| `read-file` | Empty | Registered regular file's bytes |
| `write-file` | Bytes to write | `{"bytes_written":N}`; creates a new file and never overwrites an existing one |
| `http` | Empty for GET/HEAD, otherwise raw request body | 2xx response body; non-2xx, redirects, and timeouts return Failed |
| `clock` | Empty | `{"unix_ms":N}` |
| `random` | JSON such as `{"bytes":16}` | Requested number of OS random bytes |
| `notify` | Bytes to deliver | `{"bytes_written":N}`; the CLI sink is stderr |

HTTP supports GET, HEAD, POST, PUT, PATCH, and DELETE. It uses a fixed registered URL and never follows redirects.
Input blobs cannot choose URLs, file paths, methods, headers, notification sinks, or new handles. Host registration
defines permission. File readers follow symlinks to regular files. Registered paths are trusted Host configuration;
this does not implement a filesystem sandbox.

## Configuration

```json
{
  "blobs": { "empty": "", "message": "Hello\n", "random-size": "{\"bytes\":16}" },
  "blob_files": { "binary-input": "input.bin" },
  "services": {
    "reader": { "kind": "read-file", "path": "input.bin" },
    "writer": { "kind": "write-file", "path": "new-output.bin" },
    "web": {
      "kind": "http", "method": "POST", "url": "https://example.com/api",
      "headers": {
        "Content-Type": "application/json",
        "Authorization": { "env": "API_TOKEN", "prefix": "Bearer " }
      }
    },
    "clock": { "kind": "clock" },
    "random": { "kind": "random" },
    "notice": { "kind": "notify" }
  }
}
```

File paths resolve relative to the configuration file. `blobs` registers UTF-8 strings; `blob_files` registers binary
inputs. Header values can come from environment variables, so secrets need not be embedded in configuration files.
HTTP inherits proxy and CA configuration and keeps TLS verification enabled.

## Dispatch policy and limits

- `EffectHost::execute` processes one Runtime-owned Request selected explicitly by its caller.
- `EffectHost::run` first normalizes the entire input purely, processes top-level Requests sequentially from left to
  right, replaces each occurrence in place with its fixed response, and begins the next pure evaluation interval.
- Requests nested in payloads are not discovered automatically. A trapped or limited pure interval produces no
  usable NormalForm, so Requests from that interval are not dispatched.
- A Request's Runtime/Term identity is recorded for the Host's lifetime. Repeated dispatch returns the fixed
  observation without another operation. Distinct occurrences with identical handles/blobs remain separate operations.
- Blob IDs can be inserted only once. Original bytes never change. Replaying response Terms through
  `Runtime::evaluate_terms` does not execute external operations.
- Failure blobs contain JSON with `code`, `message`, and `effects_may_have_occurred`. HTTP timeouts and partial writes
  do not establish that nothing happened externally. There is no automatic retry or rollback.
- Host limits or a failing later pure interval do not undo earlier operations. Embedders can inspect completed
  observations with `host.observations()` after `run` fails.

Defaults: 1 MiB per blob, 32 MiB total storage, 128 Requests per Host lifetime, 32 intervals per `run`, and a 5-second
HTTP timeout. Override them through configuration `limits` or `HostLimits`. The Host checks result storage capacity
before dispatch and bounds read bodies and random output sizes.

The in-memory ledger is not a durable exactly-once guarantee. Restart-safe retries, concurrency, cancellation,
streaming, transactions, and Agent integration/history remain deferred. Notification means writing to a registered
sink; no email or messaging account integration is included. Playground displays Request construction and transitions;
it does not run this native Host from the browser.
