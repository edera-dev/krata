#!/bin/sh
set -e

retry() {
  for i in $(seq 1 10)
  do
    if "${@}"
    then
      return 0
    else
      sleep "${i}"
    fi
  done
  "${@}"
}

TAG="${1}"
shift

cd target/assets

retry gh release upload "${TAG}" --clobber ./*
