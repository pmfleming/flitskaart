#!/usr/bin/env bash
# Download the Dutch Piper voices the app offers, pinned and checksummed.
# They go to $DUTCH_FLASHCARDS_VOICES, else the app's data folder
# (~/.local/share/dutch-flashcards/voices). About 250 MB in total.
set -euo pipefail

data=${DUTCH_FLASHCARDS_DATA:-${XDG_DATA_HOME:-$HOME/.local/share}/dutch-flashcards}
dest=${DUTCH_FLASHCARDS_VOICES:-$data/voices}
revision=04ca88f5685ca5253c701af3687e155b113bd512
base="https://huggingface.co/rhasspy/piper-voices/resolve/$revision/nl"
mkdir -p "$dest"

# path-in-repo  sha256
while read -r path sum; do
  file=$dest/$(basename "$path")
  if [ -s "$file" ] && echo "$sum  $file" | sha256sum --check --status; then
    echo "have  $(basename "$path")"
    continue
  fi
  echo "fetch $(basename "$path")"
  curl -fL --progress-bar -o "$file.partial" "$base/$path"
  echo "$sum  $file.partial" | sha256sum --check --status || { echo "checksum mismatch: $path" >&2; rm -f "$file.partial"; exit 1; }
  mv "$file.partial" "$file"
done <<'EOF'
nl_NL/pim/medium/nl_NL-pim-medium.onnx 1e1ed370c3b60ca734e525bdcd1f05f71b42bfb0a15693f1e89c012d72052d2a
nl_NL/pim/medium/nl_NL-pim-medium.onnx.json eaf09475a6b62191e2f3045202d63b9eb77d5f9769cf2de2f12d4d3c046bf3ac
nl_NL/ronnie/medium/nl_NL-ronnie-medium.onnx c6b69203247c9ff9aaadf59ec51147fffb16bb2c2892eb92d5b75d2f9d096622
nl_NL/ronnie/medium/nl_NL-ronnie-medium.onnx.json bc75a8810d25fe8e12f44b70f5964913ab7ffbc253cf342d82ed8f112cc527b8
nl_NL/alex/medium/nl_NL-alex-medium.onnx 758fa64ec461545972fc3e509511b48849c6f3f5c821ad55c71d9741d4352bdc
nl_NL/alex/medium/nl_NL-alex-medium.onnx.json 01eb08f80975aa4643c267abed81e6289463bad77be7b5a96d1773385586519b
nl_BE/nathalie/medium/nl_BE-nathalie-medium.onnx 2f12aea027806c24a0e9fb00080d2ee4cb0954604cc74e2fcdfe40b5d1015a09
nl_BE/nathalie/medium/nl_BE-nathalie-medium.onnx.json eb3acc456a61317d72c34a5ca97a047e60a6e7b8b31a9b9cb9318d5217374034
EOF
echo "Voices are in $dest"
