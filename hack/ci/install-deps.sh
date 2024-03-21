#!/bin/sh
set -e

sudo apt-get update
sudo apt-get install -y \
    build-essential libssl-dev libelf-dev musl-dev \
    flex bison bc protobuf-compiler musl-tools qemu-utils
sudo gem install --no-document fpm
cargo install cross
