#!/bin/sh
set -e

TOOLS_DIR="$(dirname "${0}")"
RUST_TARGET="$("${TOOLS_DIR}/target.sh")"
CROSS_COMPILE="$("${TOOLS_DIR}/cross-compile.sh")"

if [ -z "${CARGO}" ]
then
  if [ "${CROSS_COMPILE}" = "1" ] && command -v cross > /dev/null
  then
    CARGO="cross"
  else
    CARGO="cargo"
  fi
fi

export CARGO_BUILD_TARGET="${RUST_TARGET}"
exec "${CARGO}" "${@}"
