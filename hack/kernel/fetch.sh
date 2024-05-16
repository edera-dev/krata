#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="${PWD}"
cd "${KRATA_DIR}"

TARGET_ARCH="$(./hack/build/arch.sh)"

if [ "${1}" != "-u" ] && [ -f "target/kernel/kernel-${TARGET_ARCH}" ]
then
  exit 0
fi

export TARGET_ARCH
exec ./hack/build/cargo.sh run -q --bin build-fetch-kernel ghcr.io/edera-dev/kernels:latest
