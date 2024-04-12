#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."

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
  sudo mkdir -p /var/lib/krata/guest
  if [ "${KRATA_BUILD_INITRD}" = "1" ]
  then
    TARGET_ARCH="$(./hack/build/arch.sh)"
    ./hack/initrd/build.sh ${CARGO_BUILD_FLAGS}
    sudo cp "target/initrd/initrd-${TARGET_ARCH}" "/var/lib/krata/guest/initrd"
  fi
  RUST_TARGET="$(./hack/build/target.sh)"
  ./hack/build/cargo.sh build ${CARGO_BUILD_FLAGS} --bin "${EXE_TARGET}"
  exec sudo sh -c "RUST_LOG='${RUST_LOG}' 'target/${RUST_TARGET}/debug/${EXE_TARGET}' $*"
}
