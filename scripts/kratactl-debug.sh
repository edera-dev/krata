#!/bin/sh
set -e

if [ -z "${RUST_LOG}" ]
then
  RUST_LOG="INFO"
fi

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/.."
./initrd/build.sh -q
sudo cp "target/initrd/initrd" "/var/lib/krata/default/initrd"
cargo build -q --target x86_64-unknown-linux-gnu --bin kratactl
exec sudo RUST_LOG="${RUST_LOG}" target/x86_64-unknown-linux-gnu/debug/kratactl "${@}"
