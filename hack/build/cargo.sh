#!/bin/sh
set -e

TOOLS_DIR="$(dirname "${0}")"
RUST_TARGET="$("${TOOLS_DIR}/target.sh")"

if [ "${RUST_LIBC}" = "musl" ] && [ -f "/etc/alpine-release" ]
then
  export RUSTFLAGS="-Ctarget-feature=-crt-static"
fi

export CARGO_BUILD_TARGET="${RUST_TARGET}"
exec cargo "${@}"
