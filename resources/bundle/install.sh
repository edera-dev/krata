#!/bin/sh
set -e

remove_service_if_exists() {
  if systemctl show -P FragmentPath "${1}" > /dev/null
  then
    UNIT_PATH="$(systemctl show -P FragmentPath "${1}")"
    if [ -f "${UNIT_PATH}" ]
    then
      echo "[WARN] disabling and removing systemd unit ${UNIT_PATH}" > /dev/stderr
      systemctl disable --now "${1}" || true
      rm "${UNIT_PATH}"
    fi
  fi
}

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")"

remove_service_if_exists kratad.service
remove_service_if_exists kratanet.service

cp kratad.service /usr/lib/systemd/system/kratad.service
cp kratanet.service /usr/lib/systemd/system/kratanet.service

cp kratad kratanet /usr/sbin
cp kratactl /usr/bin

chmod +x /usr/sbin/kratad
chmod +x /usr/sbin/kratanet
chmod +x /usr/bin/kratactl

mkdir -p /var/lib/krata /usr/share/krata/guest
cp kernel /usr/share/krata/guest/kernel
cp initrd /usr/share/krata/guest/initrd

systemctl daemon-reload
systemctl enable kratad.service kratanet.service
systemctl restart kratad.service kratanet.service
