#!/bin/sh
set -e

if [ -z "${RUST_LIBC}" ]
then
  RUST_LIBC="gnu"
fi

if [ -z "${RUST_TARGET}" ]
then
  HOST_ARCH="$(uname -m)"
  if [ "${HOST_ARCH}" = "x86_64" ]
  then
    RUST_TARGET="x86_64-unknown-linux-${RUST_LIBC}"
  fi

  if [ "${HOST_ARCH}" = "aarch64" ]
  then
    RUST_TARGET="aarch64-unknown-linux-${RUST_LIBC}"
  fi

fi

if [ -z "${RUST_TARGET}" ]
then
  echo "ERROR: Unable to determine RUST_TARGET, your architecture may not be supported by krata." > /dev/stderr
  exit 1
fi

echo "${RUST_TARGET}"
