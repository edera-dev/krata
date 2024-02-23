#!/bin/sh
set -e

if [ -z "${RUST_LOG}" ]
then
  RUST_LOG="INFO"
fi

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/.."
cargo build --target x86_64-unknown-linux-gnu --bin kratanet
exec sudo RUST_LOG="${RUST_LOG}" target/x86_64-unknown-linux-gnu/debug/kratanet "${@}"
