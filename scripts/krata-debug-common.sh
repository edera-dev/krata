#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/.."

if [ -z "${RUST_LOG}" ]
then
  RUST_LOG="INFO"
fi

CARGO_BUILD_FLAGS=""

if [ "${KRATA_BUILD_QUIET}" = "1" ]
then
  CARGO_BUILD_FLAGS="-q"
fi

build_and_run() {
  EXE_TARGET="${1}"
  shift
  if [ "${KRATA_BUILD_INITRD}" = "1" ]
  then
    ./initrd/build.sh -q
    sudo cp "initrd/target/initrd" "/var/lib/krata/default/initrd"
  fi
  RUST_TARGET="$(./scripts/detect-rust-target.sh)"
  ./scripts/cargo.sh build ${CARGO_BUILD_FLAGS} --bin "${EXE_TARGET}"
  exec sudo RUST_LOG="${RUST_LOG}" "target/${RUST_TARGET}/debug/${EXE_TARGET}" "${@}"
}
