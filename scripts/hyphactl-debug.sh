#!/bin/sh
set -e

if [ -z "${RUST_LOG}" ]
then
  RUST_LOG="INFO"
fi

cd "$(dirname "${0}")/.."
./initrd/build.sh
sudo cp "target/initrd/initrd" "/var/lib/hypha/default/initrd"
cargo build --target x86_64-unknown-linux-gnu --bin hyphactl
exec sudo RUST_LOG="${RUST_LOG}" target/x86_64-unknown-linux-gnu/debug/hyphactl "${@}"
