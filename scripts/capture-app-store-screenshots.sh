#!/usr/bin/env bash
# Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Captures the four Mac App Store screenshots at exactly 1440x900. No window
# automation and no Accessibility permission: the window is captured at
# whatever size it happens to be (click-selected, shadow excluded), then
# centre-cropped to the 8:5 aspect ratio and resized to the exact target, so
# nothing is stretched regardless of the window's actual proportions.

set -euo pipefail

OUT="$HOME/Desktop/nostos-screenshots"
mkdir -p "$OUT"

capture_one() {
  local name="$1" label="$2"
  echo
  echo "--- $label ---"
  read -r -p "Prepara questa schermata in Nostos, poi premi Invio... "
  echo "Clicca sulla finestra di Nostos quando il cursore diventa una fotocamera."
  local raw="$OUT/${name}-raw.png"
  local file="$OUT/$name.png"
  screencapture -o -w "$raw"
  if [ ! -f "$raw" ]; then
    echo "Cattura annullata, riprovo tra un momento." >&2
    return 1
  fi

  local w h
  w=$(sips -g pixelWidth "$raw" | awk '/pixelWidth/{print $2}')
  h=$(sips -g pixelHeight "$raw" | awk '/pixelHeight/{print $2}')

  # Crop to 8:5 (1440x900's ratio) centred, then resize to the exact target,
  # so a window that was not quite in that proportion is not stretched.
  local target_h_for_w=$(( w * 5 / 8 ))
  local crop_w crop_h
  if [ "$target_h_for_w" -le "$h" ]; then
    crop_w=$w; crop_h=$target_h_for_w
  else
    crop_h=$h; crop_w=$(( h * 8 / 5 ))
  fi

  sips -c "$crop_h" "$crop_w" "$raw" --out "$raw" >/dev/null
  sips -z 900 1440 "$raw" --out "$file" >/dev/null
  rm -f "$raw"

  local dims
  dims=$(sips -g pixelWidth -g pixelHeight "$file" 2>/dev/null | awk '/pixelWidth|pixelHeight/{printf "%sx", $2}' | sed 's/x$//')
  echo "Salvato: $file  ($dims)"
}

capture_one "01-scan"       "Scansione: l'export con migliaia di foto senza date"
capture_one "02-albums"     "Vista album"
capture_one "03-duplicates" "Confronto duplicati"
capture_one "04-freespace"  "Controllo spazio libero"

echo
echo "Fatto. Screenshot in $OUT"
open "$OUT"
