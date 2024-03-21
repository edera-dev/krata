#!/bin/sh
set -e

TOOLS_DIR="$(dirname "${0}")"

RUST_TARGET="$("${TOOLS_DIR}/target.sh")"
TARGET_ARCH="$(echo "${RUST_TARGET}" | awk -F '-' '{print $1}')"
HOST_ARCH="$(uname -m)"

if [ "${HOST_ARCH}" != "${TARGET_ARCH}" ]
then
  echo "1"
else
  echo "0"
fi
