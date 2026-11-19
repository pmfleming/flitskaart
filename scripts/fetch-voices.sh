#!/usr/bin/env bash
# Download the Dutch Piper voices the app offers, pinned and checksummed.
# They go to $DUTCH_FLASHCARDS_VOICES, else the app's data folder
# (~/.local/share/dutch-flashcards/voices). About 330 MB in total.
set -euo pipefail

data=${DUTCH_FLASHCARDS_DATA:-${XDG_DATA_HOME:-$HOME/.local/share}/dutch-flashcards}
dest=${DUTCH_FLASHCARDS_VOICES:-$data/voices}
revision=04ca88f5685ca5253c701af3687e155b113bd512
base="https://huggingface.co/rhasspy/piper-voices/resolve/$revision/nl/nl_NL"
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
pim/medium/nl_NL-pim-medium.onnx 1e1ed370c3b60ca734e525bdcd1f05f71b42bfb0a15693f1e89c012d72052d2a
pim/medium/nl_NL-pim-medium.onnx.json eaf09475a6b62191e2f3045202d63b9eb77d5f9769cf2de2f12d4d3c046bf3ac
ronnie/medium/nl_NL-ronnie-medium.onnx c6b69203247c9ff9aaadf59ec51147fffb16bb2c2892eb92d5b75d2f9d096622
ronnie/medium/nl_NL-ronnie-medium.onnx.json bc75a8810d25fe8e12f44b70f5964913ab7ffbc253cf342d82ed8f112cc527b8
alex/medium/nl_NL-alex-medium.onnx 758fa64ec461545972fc3e509511b48849c6f3f5c821ad55c71d9741d4352bdc
alex/medium/nl_NL-alex-medium.onnx.json 01eb08f80975aa4643c267abed81e6289463bad77be7b5a96d1773385586519b
mls/medium/nl_NL-mls-medium.onnx 88312e0fbf505b87caf2373d94c1384892e86b1bf2ee482cf65dc8ba179cc7d3
mls/medium/nl_NL-mls-medium.onnx.json 6ddb215d38f1392ab935ad45441b82ada1eeae0452a2d6849ed71ea4f2e0aa63
mls_5809/low/nl_NL-mls_5809-low.onnx b7860ada7fa27e5e29b08e7531b0e0d74b3f0602e466a22ff95f59f80a0c4327
mls_5809/low/nl_NL-mls_5809-low.onnx.json d2192a4ed90b26166243be2cef9d71bcc4fbbbaa9461c413050bac758fafc1c3
EOF
echo "Voices are in $dest"
