#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="${PWD}"
cd "${KRATA_DIR}"

HOST_RUST_TARGET="$(TARGET_ARCH="" TARGET_LIBC="" ./hack/build/target.sh)"
TARGET_ARCH="$(./hack/build/arch.sh)"

if [ "${1}" != "-u" ] && [ -f "target/kernel/kernel-${TARGET_ARCH}" ]
then
  exit 0
fi

export TARGET_ARCH
TARGET_ARCH="" TARGET_LIBC="" RUST_TARGET="${HOST_RUST_TARGET}" ./hack/build/cargo.sh build -q --bin build-fetch-kernel
exec "target/${HOST_RUST_TARGET}/debug/build-fetch-kernel" "ghcr.io/edera-dev/kernels:latest"
