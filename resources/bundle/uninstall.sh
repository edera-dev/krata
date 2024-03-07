#!/bin/sh
set -e

systemctl disable --now kratad.service || true
systemctl disable --now kratanet.service || true

rm -f /usr/lib/systemd/system/kratad.service
rm -f /usr/lib/systemd/system/kratanet.service

rm -f /usr/bin/kratactl
rm -f /usr/libexec/kratad /usr/libexec/kratanet
rm -rf /usr/share/krata
