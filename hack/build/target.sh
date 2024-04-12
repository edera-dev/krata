#!/bin/sh
set -e

if [ -z "${TARGET_LIBC}" ] && [ -e "/etc/alpine-release" ] && [ "${KRATA_TARGET_IGNORE_LIBC}" != "1" ]
then
  TARGET_LIBC="musl"
  TARGET_VENDOR="alpine"
fi

if [ -z "${TARGET_VENDOR}" ]
then
  TARGET_VENDOR="unknown"
fi

if [ -z "${TARGET_LIBC}" ] || [ "${KRATA_TARGET_IGNORE_LIBC}" = "1" ]
then
  TARGET_LIBC="gnu"
fi

if [ -z "${TARGET_ARCH}" ]
then
  TARGET_ARCH="$(uname -m)"
fi

if [ "${TARGET_ARCH}" = "arm64" ]
then
  TARGET_ARCH="aarch64"
fi

if [ -z "${TARGET_OS}" ]
then
  TARGET_OS="$(uname -s)"
  TARGET_OS="$(echo "${TARGET_OS}" | awk -F '_' '{print $1}')"
  TARGET_OS="$(echo "${TARGET_OS}" | tr '[:upper:]' '[:lower:]')"

  if [ "${TARGET_OS}" = "mingw64" ]
  then
    TARGET_OS="windows"
  fi
fi

if [ "${TARGET_OS}" = "darwin" ]
then
  if [ -z "${RUST_TARGET}" ]
  then
    [ "${TARGET_ARCH}" = "x86_64" ] && RUST_TARGET="x86_64-apple-darwin"
    [ "${TARGET_ARCH}" = "aarch64" ] && RUST_TARGET="aarch64-apple-darwin"
  fi
elif [ "${TARGET_OS}" = "windows" ]
then
  if [ -z "${RUST_TARGET}" ]
  then
    [ "${TARGET_ARCH}" = "x86_64" ] && RUST_TARGET="x86_64-pc-windows-msvc"
    [ "${TARGET_ARCH}" = "aarch64" ] && RUST_TARGET="aarch64-pc-windows-msvc"
  fi
elif [ "${TARGET_OS}" = "freebsd" ]
then
  if [ -z "${RUST_TARGET}" ]
  then
    [ "${TARGET_ARCH}" = "x86_64" ] && RUST_TARGET="x86_64-${TARGET_VENDOR}-freebsd"
  fi
elif [ "${TARGET_OS}" = "netbsd" ]
then
  if [ -z "${RUST_TARGET}" ]
  then
    [ "${TARGET_ARCH}" = "x86_64" ] && RUST_TARGET="x86_64-${TARGET_VENDOR}-netbsd"
  fi
else
  if [ -z "${RUST_TARGET}" ]
  then
    [ "${TARGET_ARCH}" = "x86_64" ] && RUST_TARGET="x86_64-${TARGET_VENDOR}-linux-${TARGET_LIBC}"
    [ "${TARGET_ARCH}" = "aarch64" ] && RUST_TARGET="aarch64-${TARGET_VENDOR}-linux-${TARGET_LIBC}"
    [ "${TARGET_ARCH}" = "riscv64gc" ] && RUST_TARGET="riscv64gc-${TARGET_VENDOR}-linux-${TARGET_LIBC}"
  fi
fi

if [ -z "${C_TARGET}" ]
then
  [ "${TARGET_ARCH}" = "x86_64" ] && C_TARGET="x86_64-linux-${TARGET_LIBC}"
  [ "${TARGET_ARCH}" = "aarch64" ] && C_TARGET="aarch64-linux-${TARGET_LIBC}"
fi

if [ "${KRATA_TARGET_C_MODE}" = "1" ]
then
  if [ -z "${C_TARGET}" ]
  then
    echo "ERROR: Unable to determine C_TARGET, your os or architecture may not be supported by krata." > /dev/stderr
    exit 1
  fi

  echo "${C_TARGET}"
else
  if [ -z "${RUST_TARGET}" ]
  then
    echo "ERROR: Unable to determine RUST_TARGET, your os or architecture may not be supported by krata." > /dev/stderr
    exit 1
  fi

  echo "${RUST_TARGET}"
fi
