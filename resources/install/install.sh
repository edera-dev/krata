#!/bin/sh
set -e

REAL_SCRIPT="$(realpath "${0}")"
cd "$(dirname "${REAL_SCRIPT}")"

if [ -f "/etc/systemd/system/kratad.service" ]
then
  systemctl disable --now kratad.service
fi

if [ -f "/etc/systemd/system/kratanet.service" ]
then
  systemctl disable --now kratanet.service
fi

cp kratad.service /etc/systemd/system/kratad.service
cp kratanet.service /etc/systemd/system/kratanet.service

cp kratad kratanet kratactl /usr/local/bin

chmod +x /usr/local/bin/kratad
chmod +x /usr/local/bin/kratanet
chmod +x /usr/local/bin/kratactl

mkdir -p /var/lib/krata/default
cp kernel /var/lib/krata/default/kernel
cp initrd /var/lib/krata/default/initrd

systemctl daemon-reload
systemctl enable kratad.service kratanet.service
systemctl restart kratad.service kratanet.service
