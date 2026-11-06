#!/bin/sh
# Build Paint.RS and make the Linux packages in dist/:
#   dist/paint-rs_<version>_amd64.deb      (Debian, Ubuntu, Linux Mint, Pop!_OS, ...)
#   dist/paint-rs-<version>-1.x86_64.rpm   (Fedora, openSUSE, RHEL, AlmaLinux, Rocky, ...)
set -e
cd "$(dirname "$0")"
cargo build --release
cd packaging
cargo run --release --quiet
