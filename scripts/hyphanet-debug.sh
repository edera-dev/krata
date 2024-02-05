#!/bin/sh
set -e

if [ -z "${RUST_LOG}" ]
then
  RUST_LOG="INFO"
fi

cd "$(dirname "${0}")/.."
cargo build --target x86_64-unknown-linux-gnu --bin hyphanet
exec sudo RUST_LOG="${RUST_LOG}" target/x86_64-unknown-linux-gnu/debug/hyphanet "${@}"
