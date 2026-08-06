#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 || ! $1 =~ ^[a-zA-Z0-9_.-]+$ || ! $2 =~ ^[a-zA-Z0-9_.-]+$ ]]; then
  echo "usage: $0 <target> <version>" >&2
  exit 2
fi

target="$1"
version="$2"
name="sayo-${version}-${target}"

mkdir -p "dist/${name}"
install -m 755 "target/${target}/release/sayo" "dist/${name}/sayo"
install -m 644 README.md LICENSE "dist/${name}/"
tar -C dist -czf "dist/${name}.tar.gz" "$name"
