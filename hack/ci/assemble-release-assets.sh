#!/bin/sh
set -e

checksum_sha256() {
  if type sha256sum > /dev/null 2>&1
  then
    sha256sum "${1}"
  else
    shasum -a 256 "${1}"
  fi
}

asset() {
  cp "${1}" "${2}"
  PREVIOUS="${PWD}"
  cd "$(dirname "${2}")"
  BASE_FILE_NAME="$(basename "${2}")"
  checksum_sha256 "${BASE_FILE_NAME}" > "${BASE_FILE_NAME}.sha256"
  cd "${PREVIOUS}"
}

FORM="${1}"
shift
TAG_NAME="${1}"
shift
PLATFORM="${1}"
shift

mkdir -p target/assets

for SOURCE_FILE_PATH in "${@}"
do
  if [ "${FORM}" = "kratactl" ]
  then
    SUFFIX=""
    if echo "${PLATFORM}" | grep "^windows-" > /dev/null
    then
      SUFFIX=".exe"
    fi
    asset "${SOURCE_FILE_PATH}" "target/assets/kratactl_${TAG_NAME}_${PLATFORM}${SUFFIX}"
  elif [ "${FORM}" = "debian" ]
  then
    asset "${SOURCE_FILE_PATH}" "target/assets/krata_${TAG_NAME}_${PLATFORM}.deb"
  elif [ "${FORM}" = "alpine" ]
  then
    asset "${SOURCE_FILE_PATH}" "target/assets/krata_${TAG_NAME}_${PLATFORM}.apk"
  elif [ "${FORM}" = "bundle-systemd" ]
  then
    asset "${SOURCE_FILE_PATH}" "target/assets/krata-systemd_${TAG_NAME}_${PLATFORM}.tgz"
  elif [ "${FORM}" = "os" ]
  then
    asset "${SOURCE_FILE_PATH}" "target/assets/krata_${TAG_NAME}_${PLATFORM}.qcow2"
  else
    echo "ERROR: Unknown form '${FORM}'"
    exit 1
  fi
done
