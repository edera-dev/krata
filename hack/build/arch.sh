#!/bin/sh
set -e

TOOLS_DIR="$(dirname "${0}")"

RUST_TARGET="$("${TOOLS_DIR}/target.sh")"
TARGET_ARCH="$(echo "${RUST_TARGET}" | awk -F '-' '{print $1}')"

if  [ "${KRATA_ARCH_ALT_NAME}" = "1" ] || [ "${KRATA_ARCH_KERNEL_NAME}" = "1" ]
then
  if [ "${TARGET_ARCH}" = "x86_64" ] && [ "${KRATA_ARCH_KERNEL_NAME}" != "1" ]
  then
    TARGET_ARCH="amd64"
  fi

  if [ "${TARGET_ARCH}" = "aarch64" ]
  then
    TARGET_ARCH="arm64"
  fi
fi

echo "${TARGET_ARCH}"
