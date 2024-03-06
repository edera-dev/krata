#!/usr/bin/env bash
set -e

export RUST_LIBC="musl"
RUST_TARGET="$(./scripts/detect-rust-target.sh)"

export RUSTFLAGS="-Ctarget-feature=+crt-static"
cd "$(dirname "${0}")/.."
KRATA_DIR="${PWD}"
./scripts/cargo.sh build -q --release --bin krataguest
INITRD_DIR="$(mktemp -d /tmp/krata-initrd.XXXXXXXXXXXXX)"
cp "target/${RUST_TARGET}/release/krataguest" "${INITRD_DIR}/init"
chmod +x "${INITRD_DIR}/init"
cd "${INITRD_DIR}"
mkdir -p "${KRATA_DIR}/initrd/target"
find . | cpio -R 0:0 --reproducible -o -H newc --quiet > "${KRATA_DIR}/initrd/target/initrd"
rm -rf "${INITRD_DIR}"
