#!/bin/sh
set -e

TOOLS_DIR="$(dirname "${0}")"

RUST_TARGET="$("${TOOLS_DIR}/target.sh")"
TARGET_ARCH="$(echo "${RUST_TARGET}" | awk -F '-' '{print $1}')"

HOST_ARCH="$(uname -m)"

if [ "${HOST_ARCH}" = "arm64" ]
then
  HOST_ARCH="aarch64"
fi

HOST_OS="$(uname -s)"
HOST_OS="$(echo "${HOST_OS}" | awk -F '_' '{print $1}')"
HOST_OS="$(echo "${HOST_OS}" | tr '[:upper:]' '[:lower:]')"

if [ "${HOST_OS}" = "mingw64" ]
then
  HOST_OS="windows"
fi

if [ -z "${TARGET_OS}" ]
then
  TARGET_OS="${HOST_OS}"
fi

# Darwin can cross compile on all architectures to all other supported
# architectures without cross compilation consideration. For cross-compile
# check, make sure HOST_ARCH is TARGET_ARCH for comparison.
if [ "${TARGET_OS}" = "darwin" ]
then
  HOST_ARCH="${TARGET_ARCH}"
fi

if [ "${HOST_ARCH}" != "${TARGET_ARCH}" ] || [ "${HOST_OS}" != "${TARGET_OS}" ]
then
  echo "1"
else
  echo "0"
fi
