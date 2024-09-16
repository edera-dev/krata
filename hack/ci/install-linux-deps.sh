#!/bin/bash
set -e

CROSS_RS_REV="7b79041c9278769eca57fae10c74741f5aa5c14b"

PACKAGES=(build-essential musl-dev musl-tools)

sudo apt-get update

if [ "${TARGET_ARCH}" = "aarch64" ]
then
  PACKAGES+=(gcc-aarch64-linux-gnu)
fi

sudo apt-get install -y "${PACKAGES[@]}"

CROSS_COMPILE="$(./hack/build/cross-compile.sh)"

if [ "${CROSS_COMPILE}" = "1" ]
then
  cargo install cross --git "https://github.com/cross-rs/cross.git" --rev "${CROSS_RS_REV}"
fi
