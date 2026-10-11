#!/bin/bash
# «Möbel aus Foto»: install TRELLIS.2 for Apple Silicon (trellis-mac) on the server, natively (Docker has no
# Metal/MPS access). Idempotent. Run on the server:   bash ~/innenarchitekt/bin/photo3d-setup.sh
#
# Afterwards set PHOTO3D_DIR=$HOME/innenarchitekt/trellis-mac in ~/innenarchitekt/.env and redeploy/restart.
#
# Weights (~15 GB) download on first use into ~/.cache/huggingface. Two of them are gated – request access once
# with the Hugging Face account whose token is in ~/.cache/huggingface/token:
#   https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m
#   https://huggingface.co/briaai/RMBG-2.0   (CC BY-NC 4.0 – private use only)
set -euo pipefail

BASE="${IA_BASE:-$HOME/innenarchitekt}"
DIR="$BASE/trellis-mac"
PY="${PYTHON:-/opt/homebrew/bin/python3.11}"

if [ ! -d "$DIR" ]; then
  git clone https://github.com/shivampkumar/trellis-mac.git "$DIR"
fi
cd "$DIR"
# torch wheels lag behind the newest Python: create the venv with 3.11 so setup.sh reuses it
[ -d .venv ] || "$PY" -m venv .venv
# 1) Base install (clones deps, torch, patches) without the Metal backends.
SKIP_METAL=1 nice -n 10 bash setup.sh

# 2) Metal backends for fast texture baking – need Xcode (+ Metal Toolchain) and Eigen. Torch ≥ 2.14 headers
#    require C++20, the packages ask for C++17 → patch. Without Xcode: Python KDTree baker (slower, fine).
XCODE=/Applications/Xcode.app/Contents/Developer
if [ -d "$XCODE" ]; then
  export DEVELOPER_DIR="$XCODE" MACOSX_DEPLOYMENT_TARGET=12.0
  xcrun metal --version >/dev/null 2>&1 || xcodebuild -downloadComponent MetalToolchain
  [ -d /opt/homebrew/include/eigen3 ] || /opt/homebrew/bin/brew install eigen
  export CPATH=/opt/homebrew/include/eigen3
  sed -i '' 's/-std=c++17/-std=c++20/g' deps/*/setup.py deps/trellis2-apple/o-voxel/setup.py
  .venv/bin/pip install -q setuptools wheel pybind11
  for d in deps/mtlbvh deps/mtldiffrast deps/mtlmesh deps/mtlgemm deps/trellis2-apple/o-voxel; do
    nice -n 10 .venv/bin/pip install -q --no-build-isolation "$d" || echo "  $d: Build fehlgeschlagen – Fallback"
  done
else
  echo "Kein Xcode – Texturen werden mit dem langsameren Python-Baker gebacken."
fi

# Our patch: make the pre-bake simplification target configurable (default upstream: 200'000 faces).
if ! grep -q IA_TARGET_FACES generate.py; then
  sed -i '' 's/target_faces = min(200000, /target_faces = min(int(os.environ.get("IA_TARGET_FACES", "200000")), /' generate.py
fi
grep -c IA_TARGET_FACES generate.py >/dev/null && echo "✓ generate.py gepatcht (IA_TARGET_FACES)"
echo "✓ Fertig. PHOTO3D_DIR=$DIR in $BASE/.env setzen."
