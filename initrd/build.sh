#!/usr/bin/env bash
set -e

TARGET="x86_64-unknown-linux-gnu"

export RUSTFLAGS="-Ctarget-feature=+crt-static"
cd "$(dirname "${0}")/.."
krata_DIR="${PWD}"
cargo build -q --bin kratactr --release --target "${TARGET}"
INITRD_DIR="$(mktemp -d /tmp/krata-initrd.XXXXXXXXXXXXX)"
cp "target/${TARGET}/release/kratactr" "${INITRD_DIR}/init"
chmod +x "${INITRD_DIR}/init"
cd "${INITRD_DIR}"
mkdir -p "${krata_DIR}/target/initrd"
find . | cpio -R 0:0 --reproducible -o -H newc --quiet > "${krata_DIR}/target/initrd/initrd"
rm -rf "${INITRD_DIR}"
