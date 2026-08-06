#!/bin/sh
set -eu

cargo build --release --locked
mkdir -p bin
install -m 0755 target/release/herdr-cadence bin/herdr-cadence
printf 'Built %s\n' "$(pwd)/bin/herdr-cadence"
