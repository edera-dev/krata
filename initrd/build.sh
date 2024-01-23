#!/usr/bin/env bash
set -e

export RUSTFLAGS="-Ctarget-feature=+crt-static"
cd "$(dirname "${0}")/.."
HYPHA_DIR="${PWD}"
cargo build --release --target x86_64-unknown-linux-gnu
INITRD_DIR="$(mktemp -d /tmp/hypha-initrd.XXXXXXXXXXXXX)"
cp target/x86_64-unknown-linux-gnu/release/hyphactr "${INITRD_DIR}/init"
cd "${INITRD_DIR}"
mkdir -p "${HYPHA_DIR}/target/initrd"
find . | cpio -o -H newc --quiet > "${HYPHA_DIR}/target/initrd/initrd"
