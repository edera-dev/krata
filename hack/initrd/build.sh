#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="${PWD}"
cd "${KRATA_DIR}"

TARGET_ARCH="$(./hack/build/arch.sh)"

export TARGET_LIBC="musl"
RUST_TARGET="$(./hack/build/target.sh)"
export RUSTFLAGS="-Ctarget-feature=+crt-static"

./hack/build/cargo.sh build "${@}" --release --bin krataguest
INITRD_DIR="$(mktemp -d /tmp/krata-initrd.XXXXXXXXXXXXX)"
cp "target/${RUST_TARGET}/release/krataguest" "${INITRD_DIR}/init"
chmod +x "${INITRD_DIR}/init"
cd "${INITRD_DIR}"
mkdir -p "${KRATA_DIR}/target/initrd"
find . | cpio -R 0:0 --ignore-devno --renumber-inodes -o -H newc --quiet > "${KRATA_DIR}/target/initrd/initrd-${TARGET_ARCH}"
rm -rf "${INITRD_DIR}"
