#!/usr/bin/env bash
# Builds MuJoCo as a static library for iOS devices and the (arm64) iOS Simulator, installing
# into ios/mujoco/<iphoneos|iphonesimulator>/{include,lib/libmujoco.a}. libmujoco.a bundles
# MuJoCo's third-party dependencies (qhull, tinyxml2, ccd, lodepng, ...). Re-runs are no-ops when
# the outputs exist. Needs macOS with Xcode, CMake and Ninja.
set -euo pipefail
VERSION="${MUJOCO_VERSION:-4.0.1}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/mujoco"
SRC="${MUJOCO_SRC:-$HERE/.mujoco-src}"
SDKS="${SDKS:-iphoneos iphonesimulator}"
TARGET="${IOS_DEPLOYMENT_TARGET:-16.0}"
JOBS="$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"

if [ ! -f "$SRC/CMakeLists.txt" ]; then
  rm -rf "$SRC"
  git clone --quiet --depth 1 --branch "$VERSION" https://github.com/google-deepmind/mujoco "$SRC"
fi

# MuJoCo always builds a shared library and forces macOS-only flags on every Apple platform.
# iOS apps link it statically, and the iOS compiler must not receive -mmacosx-version-min.
patch_ios() {
  grep -q "TULPA_FIELD_IOS_PATCH" "$SRC/CMakeLists.txt" && return
  python3 - "$SRC" <<'PY'
import sys, pathlib
root = pathlib.Path(sys.argv[1])
cm = root / "CMakeLists.txt"
s = cm.read_text()
a = "  add_library(mujoco SHARED ${MUJOCO_RESOURCE_FILES})"
assert a in s, "add_library(mujoco SHARED) not found"
s = s.replace(a, "  add_library(mujoco STATIC ${MUJOCO_RESOURCE_FILES})  # TULPA_FIELD_IOS_PATCH", 1)
cm.write_text(s)
mm = root / "cmake" / "MujocoMacOS.cmake"
t = mm.read_text()
assert "if(APPLE)" in t, "MujocoMacOS.cmake layout changed"
mm.write_text(t.replace("if(APPLE)", "if(APPLE AND NOT IOS)"))
PY
}

build() {  # sdk
  local sdk="$1"
  if [ -f "$OUT/$sdk/include/mujoco/mujoco.h" ] && [ -f "$OUT/$sdk/lib/libmujoco.a" ]; then
    echo "MuJoCo $VERSION for $sdk: cached"; return
  fi
  echo "::group::MuJoCo $VERSION for $sdk"
  local b="$HERE/.mujoco-build/$sdk"
  rm -rf "$b"
  cmake -S "$SRC" -B "$b" -G Ninja -DCMAKE_BUILD_TYPE=Release \
    -DMUJOCO_BUILD_EXAMPLES=OFF -DMUJOCO_BUILD_SIMULATE=OFF -DMUJOCO_BUILD_TESTS=OFF \
    -DMUJOCO_BUILD_STUDIO=OFF -DMUJOCO_TEST_PYTHON_UTIL=OFF -DBUILD_TESTING=OFF \
    -DMUJOCO_ENABLE_AVX=OFF -DMUJOCO_ENABLE_AVX_INTRINSICS=OFF \
    -DCMAKE_SYSTEM_NAME=iOS -DCMAKE_OSX_SYSROOT="$sdk" -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$TARGET" -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF \
    -DCMAKE_MACOSX_BUNDLE=OFF -DQHULL_ENABLE_TESTING=OFF
  cmake --build "$b" --target mujoco -j"$JOBS"
  rm -rf "$OUT/$sdk"; mkdir -p "$OUT/$sdk/include" "$OUT/$sdk/lib"
  cp -r "$SRC/include/mujoco" "$OUT/$sdk/include/"
  # one archive with MuJoCo and every static dependency it was linked against
  local libs=()
  while IFS= read -r f; do libs+=("$f"); done < <(find "$b" -name '*.a' \
    ! -path '*/googletest*' ! -path '*/benchmark*' ! -path '*/abseil*' | sort)
  printf '  %s\n' "${libs[@]}"
  libtool -static -no_warning_for_no_symbols -o "$OUT/$sdk/lib/libmujoco.a" "${libs[@]}"
  echo "::endgroup::"
  ls -la "$OUT/$sdk/lib"
}

patch_ios
for sdk in $SDKS; do build "$sdk"; done
