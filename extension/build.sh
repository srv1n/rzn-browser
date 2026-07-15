#!/bin/bash
set -euo pipefail

TARGET="${1:-all}"
case "$TARGET" in
    all|chrome|edge|chromium|firefox) ;;
    *)
        echo "Unsupported extension build target: ${TARGET}" >&2
        echo "Expected one of: all, chrome, edge, chromium, firefox" >&2
        exit 2
        ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

if [ -z "${RZN_BUILD_SIGNATURE:-}" ]; then
    RZN_BUILD_SIGNATURE="$(date -u +"%Y%m%dT%H%M%SZ")"
fi
export RZN_BUILD_SIGNATURE
echo "Using RZN_BUILD_SIGNATURE=${RZN_BUILD_SIGNATURE}"

TARGETS=("$TARGET")
if [ "$TARGET" = "all" ]; then
    TARGETS=(chrome edge chromium firefox)
    rm -rf "${SCRIPT_DIR}/dist"
    mkdir -p "${SCRIPT_DIR}/dist"
fi

build_target() {
    local target="$1"
    local build_dir="${SCRIPT_DIR}/.build/${target}"
    export RZN_EXTENSION_TARGET="$target"
    export RZN_EXTENSION_OUT_DIR="$build_dir"

    echo "Building ${target} extension..."
    echo "Using RZN_EXTENSION_TARGET=${RZN_EXTENSION_TARGET}"

    rm -rf "$build_dir" "${SCRIPT_DIR}/dist/${target}"
    mkdir -p "$build_dir" "${SCRIPT_DIR}/dist"

    echo "Building background script..."
    bun x vite build --config vite.config.background.ts

    echo "Building content script..."
    bun x vite build --config vite.config.content.ts

    echo "Building shadow DOM instrumentation..."
    bun x vite build --config vite.config.shadow-dom.ts

    echo "Building page bridge (MAIN world)..."
    bun x vite build --config vite.config.pagebridge.ts

    echo "Building popup..."
    bun x vite build --config vite.config.popup.ts

    echo "Building dashboard..."
    bun x vite build --config vite.config.dashboard.ts

    echo "Renaming built files..."
    if [ -f "${build_dir}/background.iife.js" ]; then
        mv "${build_dir}/background.iife.js" "${build_dir}/background.js"
    fi
    if [ -f "${build_dir}/contentScript.iife.js" ]; then
        mv "${build_dir}/contentScript.iife.js" "${build_dir}/contentScript.js"
    fi
    if [ -f "${build_dir}/shadow-dom-instrumentation.iife.js" ]; then
        mv "${build_dir}/shadow-dom-instrumentation.iife.js" "${build_dir}/shadow-dom-instrumentation.js"
    fi
    if [ -f "${build_dir}/pageBridge.iife.js" ]; then
        mv "${build_dir}/pageBridge.iife.js" "${build_dir}/pageBridge.js"
    fi

    echo "Generating ${target} manifest..."
    (cd "$ROOT_DIR" && bun scripts/build-ext.ts \
        --target "$target" \
        --source-dir "$build_dir" \
        --dist-root "${SCRIPT_DIR}/dist" \
        --layout nested \
        --build-signature "$RZN_BUILD_SIGNATURE")

    if [ "$target" = "chrome" ]; then
        echo "Generating chrome legacy dist-chrome alias..."
        (cd "$ROOT_DIR" && bun scripts/build-ext.ts \
            --target chrome \
            --source-dir "$build_dir" \
            --dist-root "$SCRIPT_DIR" \
            --layout legacy \
            --build-signature "$RZN_BUILD_SIGNATURE")
    fi
}

for target in "${TARGETS[@]}"; do
    build_target "$target"
done

echo "Build complete!"
