#!/bin/zsh
set -euo pipefail

script_directory="${0:A:h}"
output_directory="$script_directory/.build/debug"
output_path="$output_directory/rusteze-capture-helper"

mkdir -p "$output_directory"
xcrun swiftc "$script_directory/Sources/main.swift" \
  -o "$output_path" \
  -framework AVFoundation \
  -framework CoreGraphics

print "Built $output_path"
