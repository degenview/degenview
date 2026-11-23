#!/bin/zsh
# Records, narrates and composes a tutorial video end to end.
#   tools/tutorial-video/make.sh <tutorial> [--dry]
# --dry skips the narration (and its API calls) and writes a silent preview from estimated timing.
# DEGENVIEW_BIN=<path> picks the app binary, DEGENVIEW_VIDEO_WORK=<dir> the work directory, and
# OPENAI_API_KEY (or KEY_FILE=<path>) the narration key. See tools/tutorial-video/README.md.
set -euo pipefail
here=${0:A:h}
tutorial=${1:?usage: make.sh <tutorial> [--dry]}
dry=${2:-}
cd $here
if [[ $dry != --dry ]]; then
  python3 tts.py $tutorial ${KEY_FILE:+--key-file $KEY_FILE}
fi
python3 record.py $tutorial $dry
python3 compose.py $tutorial $dry
