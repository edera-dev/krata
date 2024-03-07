#!/bin/sh
set -e

TOOLS_DIR="$(dirname "${0}")"
RUST_TARGET="$("${TOOLS_DIR}/target.sh")"

export CARGO_BUILD_TARGET="${RUST_TARGET}"
exec cargo "${@}"
