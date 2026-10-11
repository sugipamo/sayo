# File → HTTP → new file

English | [日本語](README.ja.md)

This example reads a synthetic sample file, POSTs its bytes to a loopback HTTP server, and saves a successful
response body in a new file. Sayo World relations choose the next operation or stop on failure; the existing
sequential `EffectHost::run` executes the Requests. Bodies remain opaque `Data<blob:...>` bytes. Only HTTP status
and success/failure enter the semantic envelope.

From the repository root, with a Rust build environment and Python 3:

```sh
cargo build --locked --features host-effects --bin sayo-host
python3 examples/file_http_pipeline/run.py
python3 examples/file_http_pipeline/run.py --status 503
python3 examples/file_http_pipeline/run.py --missing-input
python3 examples/file_http_pipeline/run.py --existing-output
```

The runner starts a server on an available `127.0.0.1` port, copies the checked-in `sample.txt` into a fresh
directory, writes `host.json`, runs the CLI, stops the server, and prints the directory path. HTTP exchanges use
only this loopback server. `--output-dir PATH` chooses a directory that must not already exist; `--host-bin PATH`
selects the built executable.

| Run | Final Terms | HTTP requests | Save |
| --- | --- | --- | --- |
| Default (201) | `Saved{Data<blob:...>} HttpStatus{201}` | 1 | Body saved to `response.bin` |
| `--status 503` | `HttpFailed{Data<blob:...>} HttpStatus{503}` | 1 | Skipped |
| `--missing-input` | `ReadFailed{Data<blob:...>}` | 0 | Skipped |
| `--existing-output` | `SaveFailed{Data<blob:...>} HttpStatus{201}` | 1 | Original file preserved |

The directory retains `input.txt`, `host.json`, and `observations/*.blob`. Successful and non-2xx HTTP blobs contain
raw response bodies. File, transport, and body-read/size failures contain diagnostic JSON. The `Saved` blob is a
`bytes_written` receipt; the saved body is in `response.bin`. Operation failures are Sayo results, so the CLI still
exits 0. Configuration errors, Host limits, and pure evaluation errors exit nonzero.

`main.sayo` leaves `ReadStage{}` and one read Request. `ReadStage/Succeeded/ReadReady` captures its successful
observation, and `ReadReady` passes the opaque data into the HTTP Request. A failed read ends as `ReadFailed`.
The HTTP service opts into `response_metadata: true`, producing fixed envelopes such as:

```sayo
HttpResponse{HttpStatus{201}, Succeeded{Data<blob:body>}}
HttpResponse{HttpStatus{503}, Failed{Data<blob:body>}}
HttpResponse{NoHttpStatus{}, Failed{Data<blob:diagnostic>}}
```

`HttpReady` explicitly applies `inspect` to project outcome and status. `HandleHttp/Succeeded/HttpOk` creates the
save Request; `HandleHttp/Failed/HttpError` stops as `HttpFailed`. The World's `payload` Relation projects the `Data`
Term from its envelope without inspecting blob contents. Status remains in the Term sequence while save results
branch to `Saved` or `SaveFailed`. Body-read failures retain a received status and substitute a diagnostic blob.

To use your own local configuration (file paths are relative to its directory):

```sh
target/debug/sayo-host --config PATH/host.json \
  --world examples/file_http_pipeline/world \
  --export-blobs PATH/observations examples/file_http_pipeline/main.sayo
```

Each stage creates one Request. There are no automatic retries, redirect following, concurrent effects, streaming,
or Agent integration. Alternative failure behavior can be written in the World's failure Relations. Reexecuting the
same Request in one Host returns its cached observation; another operation requires a distinct Request occurrence.

```sh
cargo run --bin sayo -- check --world examples/file_http_pipeline/world
cargo test --locked --features host-effects --test host_effects
```

Tests use synthetic binary bytes and loopback servers for successful saves, 503/302 body retention and stopping,
read failures without HTTP, existing-file preservation, timeout/body limits, and pure replay. See the
[native Host contract](../../docs/host-effects.md).
