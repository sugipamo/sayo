#!/usr/bin/env python3
"""Run the Sayo program using synthetic data and a loopback-only HTTP server."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--status", type=int, default=201)
    parser.add_argument("--missing-input", action="store_true")
    parser.add_argument("--existing-output", action="store_true")
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--host-bin", type=Path, default=Path("target/debug/sayo-host"))
    args = parser.parse_args()
    if not 200 <= args.status <= 599:
        parser.error("--status must be between 200 and 599")
    binary = args.host_bin.resolve()
    if not binary.is_file():
        parser.error("build first: cargo build --locked --features host-effects --bin sayo-host")
    root = Path(__file__).resolve().parent
    directory = args.output_dir
    if directory is None:
        directory = Path(tempfile.mkdtemp(prefix="sayo-file-http-"))
    else:
        directory = directory.resolve()
        directory.mkdir(parents=True, exist_ok=False)
    if not args.missing_input:
        shutil.copyfile(root / "sample.txt", directory / "input.txt")
    if args.existing_output:
        (directory / "response.bin").write_bytes(b"existing sample: keep me\n")
    requests = []

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 <= length <= 1_048_576:
                self.send_error(413)
                return
            body = self.rfile.read(length)
            requests.append(body)
            prefix = b"accepted sample:\n" if args.status < 300 else b"rejected sample:\n"
            response = prefix + body
            if args.status in (204, 205, 304):
                response = b""
            self.send_response(args.status)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(response)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(response)

        def log_message(self, *_):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    server.timeout = 5
    url = f"http://127.0.0.1:{server.server_port}/sample"
    config = {
        "blobs": {"empty": ""},
        "services": {
            "reader": {"kind": "read-file", "path": "input.txt"},
            "web": {
                "kind": "http", "method": "POST", "url": url,
                "response_metadata": True,
                "headers": {"Content-Type": "application/octet-stream"},
            },
            "writer": {"kind": "write-file", "path": "response.bin"},
        },
    }
    (directory / "host.json").write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    env = dict(os.environ, NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
    try:
        result = subprocess.run([
            str(binary), "--config", str(directory / "host.json"),
            "--world", str(root / "world"),
            "--export-blobs", str(directory / "observations"), str(root / "main.sayo"),
        ], env=env, timeout=30, check=False)
    finally:
        server.shutdown()
        worker.join(timeout=5)
        server.server_close()
    print(f"HTTP requests: {len(requests)}; synthetic sample only; {url}")
    print(f"Files kept in: {directory}")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
