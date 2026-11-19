#!/usr/bin/env bash
# Download what the app runs locally, pinned and checksummed:
# - the Dutch Piper voices (~250 MB) to $DUTCH_FLASHCARDS_VOICES, else <data>/voices
# - the Whisper speech recognizer (~575 MB) to $DUTCH_FLASHCARDS_MODELS, else <data>/models
# <data> is ~/.local/share/dutch-flashcards unless $DUTCH_FLASHCARDS_DATA is set.
set -euo pipefail

data=${DUTCH_FLASHCARDS_DATA:-${XDG_DATA_HOME:-$HOME/.local/share}/dutch-flashcards}
voices=${DUTCH_FLASHCARDS_VOICES:-$data/voices}
models=${DUTCH_FLASHCARDS_MODELS:-$data/models}
mkdir -p "$voices" "$models"

# fetch URL DESTINATION_DIR SHA256
fetch() {
  local url=$1 dest=$2 sum=$3
  local file=$dest/$(basename "$url")
  if [ -s "$file" ] && echo "$sum  $file" | sha256sum --check --status; then
    echo "have  $(basename "$file")"
    return
  fi
  echo "fetch $(basename "$file")"
  curl -fL --progress-bar -o "$file.partial" "$url"
  echo "$sum  $file.partial" | sha256sum --check --status || { echo "checksum mismatch: $url" >&2; rm -f "$file.partial"; exit 1; }
  mv "$file.partial" "$file"
}

piper=https://huggingface.co/rhasspy/piper-voices/resolve/04ca88f5685ca5253c701af3687e155b113bd512/nl
while read -r path sum; do
  fetch "$piper/$path" "$voices" "$sum"
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
fetch https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin \
  "$models" 394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2
echo "Voices are in $voices, the speech recognizer in $models"
