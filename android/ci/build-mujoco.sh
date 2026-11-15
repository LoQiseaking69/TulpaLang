#!/usr/bin/env bash
# Builds MuJoCo for the host (native tests) and for each Android ABI the app ships, installing
# into android/mujoco/<host|abi>/{include,lib}. Re-runs are no-ops when the outputs exist.
set -euo pipefail
VERSION="${MUJOCO_VERSION:-4.0.1}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/mujoco"
SRC="${MUJOCO_SRC:-$HERE/.mujoco-src}"
ABIS="${ABIS:-arm64-v8a armeabi-v7a x86_64}"
NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-}}"
JOBS="$(nproc 2>/dev/null || echo 4)"

common=(-G Ninja -DCMAKE_BUILD_TYPE=Release
  -DMUJOCO_BUILD_EXAMPLES=OFF -DMUJOCO_BUILD_SIMULATE=OFF -DMUJOCO_BUILD_TESTS=OFF
  -DMUJOCO_BUILD_STUDIO=OFF -DMUJOCO_TEST_PYTHON_UTIL=OFF -DBUILD_TESTING=OFF)

if [ ! -f "$SRC/CMakeLists.txt" ]; then
  rm -rf "$SRC"
  git clone --quiet --depth 1 --branch "$VERSION" https://github.com/google-deepmind/mujoco "$SRC"
fi

# Android before API 28 has no aligned_alloc: use memalign (bionic frees it with free()).
patch_android() {
  local f="$SRC/src/engine/engine_util_errmem.c"
  grep -q "TULPA_FIELD_PATCH" "$f" && return
  python3 - "$f" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
s = s.replace("#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L\n  return aligned_alloc(align, size);",
              "#elif defined(__ANDROID__)  // TULPA_FIELD_PATCH\n  return memalign(align, size);\n#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L\n  return aligned_alloc(align, size);", 1)
s = "#ifdef __ANDROID__\n#include <malloc.h>\n#endif\n" + s
open(p, "w").write(s)
PY
  grep -q "TULPA_FIELD_PATCH" "$f" || { echo "::error::could not patch $f"; exit 1; }
}

build() {  # name, extra cmake args...
  local name="$1"; shift
  if [ -f "$OUT/$name/include/mujoco/mujoco.h" ] && ls "$OUT/$name/lib/libmujoco.so" >/dev/null 2>&1; then
    echo "MuJoCo $VERSION for $name: cached"; return
  fi
  echo "::group::MuJoCo $VERSION for $name"
  local b="$HERE/.mujoco-build/$name"
  rm -rf "$b"
  cmake -S "$SRC" -B "$b" "${common[@]}" -DCMAKE_INSTALL_PREFIX="$OUT/$name" "$@"
  cmake --build "$b" --target mujoco -j"$JOBS"
  # headers + the shared library are all the bridge needs
  rm -rf "$OUT/$name"; mkdir -p "$OUT/$name/include" "$OUT/$name/lib"
  cp -r "$SRC/include/mujoco" "$OUT/$name/include/"
  find "$b" -name 'libmujoco.so*' -exec cp -P {} "$OUT/$name/lib/" \;
  echo "::endgroup::"
  ls -la "$OUT/$name/lib"
}

if [ "${SKIP_HOST:-0}" != 1 ]; then build host; fi

if [ -n "$NDK" ]; then
  patch_android
  for abi in $ABIS; do
    build "$abi" \
      -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
      -DANDROID_ABI="$abi" -DANDROID_PLATFORM=android-26 -DANDROID_STL=c++_static \
      -DCMAKE_PLATFORM_NO_VERSIONED_SONAME=ON
    # Android packages only lib*.so: the library must not carry a versioned SONAME
    if readelf -d "$OUT/$abi/lib/libmujoco.so" | grep -q 'SONAME.*libmujoco.so\.'; then
      echo "::error::$abi libmujoco.so has a versioned SONAME"; exit 1
    fi
  done
else
  echo "ANDROID_NDK_HOME not set: built the host library only"
fi
