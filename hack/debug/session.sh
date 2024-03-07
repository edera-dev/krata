#!/bin/sh
set -e

stop_service_if_running() {
  if sudo systemctl is-active "${1}" > /dev/null 2>&1
  then
    sudo systemctl stop "${1}"
  fi

}

stop_service_if_running "kratad.service"
stop_service_if_running "kratanet.service"
tmuxp load "$(dirname "${0}")/session.yml"
