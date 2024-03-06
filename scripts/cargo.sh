#!/bin/sh
set -e

SCRIPTS_DIR="$(dirname "${0}")"
RUST_TARGET="$("${SCRIPTS_DIR}/detect-rust-target.sh")"

export CARGO_BUILD_TARGET="${RUST_TARGET}"
exec cargo "${@}"
