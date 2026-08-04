#!/bin/zsh
set -euo pipefail

project_root="${0:A:h:h}"
runtime_directory="${RUSTEZE_WHISPER_CPP_DIR:-$project_root/tools/whisper.cpp}"
model_directory="${RUSTEZE_MODEL_DIR:-$project_root/models}"
model_name="${RUSTEZE_WHISPER_MODEL_NAME:-large-v3-turbo-q5_0}"
model_path="$model_directory/ggml-$model_name.bin"

if [[ ! -d "$runtime_directory/.git" ]]; then
  mkdir -p "${runtime_directory:h}"
  git clone --depth 1 https://github.com/ggml-org/whisper.cpp.git "$runtime_directory"
fi

cmake -S "$runtime_directory" -B "$runtime_directory/build" -DWHISPER_COREML=OFF
cmake --build "$runtime_directory/build" --config Release -j

mkdir -p "$model_directory"
if [[ ! -f "$model_path" ]]; then
  "$runtime_directory/models/download-ggml-model.sh" "$model_name"
  mv "$runtime_directory/models/ggml-$model_name.bin" "$model_path"
fi

cat <<EOF

whisper.cpp is ready.

Runtime: $runtime_directory/build/bin/whisper-cli
Model:   $model_path

Export these before running the future Rusteze adapter:
  export RUSTEZE_WHISPER_CPP_BIN="$runtime_directory/build/bin/whisper-cli"
  export RUSTEZE_WHISPER_MODEL="$model_path"
EOF
