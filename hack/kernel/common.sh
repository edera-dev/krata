#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")/../.."
KRATA_DIR="$(realpath "${PWD}")"
KERNEL_DIR="${KRATA_DIR}/kernel"

cd "${KRATA_DIR}"

TARGET_ARCH_STANDARD="$(KRATA_ARCH_KERNEL_NAME=0 ./hack/build/arch.sh)"
TARGET_ARCH_KERNEL="$(KRATA_ARCH_KERNEL_NAME=1 ./hack/build/arch.sh)"
C_TARGET="$(KRATA_TARGET_C_MODE=1 KRATA_TARGET_IGNORE_LIBC=1 ./hack/build/target.sh)"
IS_CROSS_COMPILE="$(./hack/build/cross-compile.sh)"

if [ "${IS_CROSS_COMPILE}" = "1" ]
then
  CROSS_COMPILE_MAKE="CROSS_COMPILE=${C_TARGET}-"
else
  CROSS_COMPILE_MAKE="CROSS_COMPILE="
fi

# shellcheck source-path=SCRIPTDIR source=../../kernel/config.sh
. "${KERNEL_DIR}/config.sh"
KERNEL_SRC="${KERNEL_DIR}/linux-${KERNEL_VERSION}-${TARGET_ARCH_STANDARD}"

if [ -z "${KRATA_KERNEL_BUILD_JOBS}" ]
then
  KRATA_KERNEL_BUILD_JOBS="$(nproc)"
fi

if [ ! -f "${KERNEL_SRC}/Makefile" ]
then
  rm -rf "${KERNEL_SRC}"
  mkdir -p "${KERNEL_SRC}"
  curl --progress-bar -L -o "${KERNEL_SRC}.txz" "${KERNEL_SRC_URL}"
  tar xf "${KERNEL_SRC}.txz" --strip-components 1 -C "${KERNEL_SRC}"
  rm "${KERNEL_SRC}.txz"
fi

OUTPUT_DIR="${KRATA_DIR}/target/kernel"
mkdir -p "${OUTPUT_DIR}"

KERNEL_CONFIG_FILE="${KERNEL_DIR}/krata-${TARGET_ARCH_STANDARD}.config"

if [ ! -f "${KERNEL_CONFIG_FILE}" ]
then
  echo "ERROR: kernel config file not found for ${TARGET_ARCH_STANDARD}" > /dev/stderr
  exit 1
fi

cp "${KERNEL_CONFIG_FILE}" "${KERNEL_SRC}/.config"
make -C "${KERNEL_SRC}" ARCH="${TARGET_ARCH_KERNEL}" "${CROSS_COMPILE_MAKE}" olddefconfig

# shellcheck disable=SC2034
IMAGE_TARGET="bzImage"

if [ "${TARGET_ARCH_STANDARD}" = "x86_64" ]
then
  # shellcheck disable=SC2034
  IMAGE_TARGET="bzImage"
elif [ "${TARGET_ARCH_STANDARD}" = "aarch64" ]
then
  # shellcheck disable=SC2034
  IMAGE_TARGET="Image.gz"
fi

# shellcheck disable=SC2034
MODULES_INSTALL_PATH="${OUTPUT_DIR}/modules-install-${TARGET_ARCH_STANDARD}"
# shellcheck disable=SC2034
ADDONS_OUTPUT_PATH="${OUTPUT_DIR}/addons-${TARGET_ARCH_STANDARD}"
# shellcheck disable=SC2034
MODULES_OUTPUT_PATH="${ADDONS_OUTPUT_PATH}/modules"
# shellcheck disable=SC2034
ADDONS_SQUASHFS_PATH="${OUTPUT_DIR}/addons-${TARGET_ARCH_STANDARD}.squashfs"
