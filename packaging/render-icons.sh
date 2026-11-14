#!/usr/bin/env bash
# Regenerates the PNG icons from the SVG sources (needs rsvg-convert).
set -euo pipefail
cd "$(dirname "$0")/../assets/icons"
for size in 22 32 44 256; do
    rsvg-convert -w "$size" -h "$size" rtools.svg -o "rtools-$size.png"
done
# macOS template image: the panel glyph in solid black; the system recolors it.
sed 's/#232629/#000000/' rtools-tray.svg | rsvg-convert -w 44 -h 44 -o rtools-tray-template-44.png
