#!/bin/sh
set -e

cd "$(dirname "${0}")"

# shellcheck source=config.sh
. "${PWD}/config.sh"

if [ ! -f "${SRC_DIR_NAME}/Makefile" ]
then
  rm -rf "${SRC_DIR_NAME}"
  curl -L -o "${SRC_DIR_NAME}.txz" "${KERNEL_SRC_URL}"
  tar xf "${SRC_DIR_NAME}.txz"
  rm "${SRC_DIR_NAME}.txz"
fi

mkdir -p "${OUTPUT_DIR_NAME}"

cp krata.config "${SRC_DIR_NAME}/.config"
make -C "${SRC_DIR_NAME}" "${@}" olddefconfig
make -C "${SRC_DIR_NAME}" "${@}" bzImage
cp "${SRC_DIR_NAME}/arch/x86/boot/bzImage" "${OUTPUT_DIR_NAME}/kernel"
