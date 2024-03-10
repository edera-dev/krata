#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="${PWD}"
KERNEL_DIR="${KRATA_DIR}/kernel"

cd "${KRATA_DIR}"

# shellcheck source-path=SCRIPTDIR source=../../kernel/config.sh
. "${KERNEL_DIR}/config.sh"
KERNEL_SRC="${KERNEL_DIR}/linux-${KERNEL_VERSION}"

if [ -z "${KRATA_KERNEL_BUILD_JOBS}" ]
then
  KRATA_KERNEL_BUILD_JOBS="2"
fi

if [ ! -f "${KERNEL_SRC}/Makefile" ]
then
  rm -rf "${KERNEL_SRC}"
  curl --progress-bar -L -o "${KERNEL_SRC}.txz" "${KERNEL_SRC_URL}"
  tar xf "${KERNEL_SRC}.txz" -C "${KERNEL_DIR}"
  rm "${KERNEL_SRC}.txz"
fi

OUTPUT_DIR="${KRATA_DIR}/target/kernel"
mkdir -p "${OUTPUT_DIR}"

TARGET_ARCH="$(KRATA_ARCH_KERNEL_NAME=1 ./hack/build/arch.sh)"

KERNEL_CONFIG_FILE="${KERNEL_DIR}/krata-${TARGET_ARCH}.config"

if [ ! -f "${KERNEL_CONFIG_FILE}" ]
then
  echo "ERROR: kernel config file not found for ${TARGET_ARCH}" > /dev/stderr
  exit 1
fi

cp "${KERNEL_CONFIG_FILE}" "${KERNEL_SRC}/.config"
make -C "${KERNEL_SRC}" ARCH="${TARGET_ARCH}" olddefconfig

if [ "${TARGET_ARCH}" = "x86_64" ]
then
  make -C "${KERNEL_SRC}" ARCH="${TARGET_ARCH}" -j"${KRATA_KERNEL_BUILD_JOBS}" bzImage
elif [ "${TARGET_ARCH}" = "arm64" ]
then
  make -C "${KERNEL_SRC}" ARCH="${TARGET_ARCH}" -j"${KRATA_KERNEL_BUILD_JOBS}" Image.gz
fi

if [ "${TARGET_ARCH}" = "x86_64" ]
then
  cp "${KERNEL_SRC}/arch/x86/boot/bzImage" "${OUTPUT_DIR}/kernel"
elif [ "${TARGET_ARCH}" = "arm64" ]
then
  cp "${KERNEL_SRC}/arch/arm64/boot/Image.gz" "${OUTPUT_DIR}/kernel"
else
  echo "ERROR: unable to determine what file is the vmlinuz for ${TARGET_ARCH}" > /dev/stderr
  exit 1
fi
