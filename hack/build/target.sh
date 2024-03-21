#!/bin/sh
set -e

if [ -z "${TARGET_LIBC}" ] || [ "${KRATA_TARGET_IGNORE_LIBC}" = "1" ]
then
  TARGET_LIBC="gnu"
fi

if [ -z "${TARGET_ARCH}" ]
then
  TARGET_ARCH="$(uname -m)"
fi

if [ -z "${RUST_TARGET}" ]
then
  if [ "${TARGET_ARCH}" = "x86_64" ]
  then
    RUST_TARGET="x86_64-unknown-linux-${TARGET_LIBC}"
  fi

  if [ "${TARGET_ARCH}" = "aarch64" ]
  then
    RUST_TARGET="aarch64-unknown-linux-${TARGET_LIBC}"
  fi
fi

if [ -z "${C_TARGET}" ]
then
  if [ "${TARGET_ARCH}" = "x86_64" ]
  then
    C_TARGET="x86_64-linux-${TARGET_LIBC}"
  fi

  if [ "${TARGET_ARCH}" = "aarch64" ]
  then
    C_TARGET="aarch64-linux-${TARGET_LIBC}"
  fi
fi

if [ "${KRATA_TARGET_C_MODE}" = "1" ]
then
  if [ -z "${C_TARGET}" ]
  then
    echo "ERROR: Unable to determine C_TARGET, your architecture may not be supported by krata." > /dev/stderr
    exit 1
  fi

  echo "${C_TARGET}"
else
  if [ -z "${RUST_TARGET}" ]
  then
    echo "ERROR: Unable to determine RUST_TARGET, your architecture may not be supported by krata." > /dev/stderr
    exit 1
  fi

  echo "${RUST_TARGET}"
fi
