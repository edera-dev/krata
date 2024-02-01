#!/usr/bin/env bash
set -e

TARGET="x86_64-unknown-linux-musl"

export RUSTFLAGS="-Ctarget-feature=+crt-static"
cd "$(dirname "${0}")/.."
HYPHA_DIR="${PWD}"
cargo build --bin hyphactr --release --target "${TARGET}"
INITRD_DIR="$(mktemp -d /tmp/hypha-initrd.XXXXXXXXXXXXX)"
cp "target/${TARGET}/release/hyphactr" "${INITRD_DIR}/init"
chmod +x "${INITRD_DIR}/init"
cd "${INITRD_DIR}"
mkdir -p "${HYPHA_DIR}/target/initrd"
find . | cpio -R 0:0 --reproducible -o -H newc --quiet > "${HYPHA_DIR}/target/initrd/initrd"
rm -rf "${INITRD_DIR}"
