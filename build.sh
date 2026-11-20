#!/bin/sh
# Builds the 64-bit module for the current host and renames it the way Garry's Mod expects.
#   Linux  -> gmsv_waterlevelercore_linux64.dll
#   Windows (run under MSYS2/Git Bash) -> gmsv_waterlevelercore_win64.dll and
#                                         gmcl_waterlevelercore_win64.dll (two builds: the server
#                                         module has only the server code, the client module only
#                                         the client code)
# Cross-compiling for Windows from Linux:
#   rustup target add x86_64-pc-windows-gnu && sudo apt install mingw-w64
#   cargo build --release --target x86_64-pc-windows-gnu --no-default-features --features server
#   cp target/x86_64-pc-windows-gnu/release/gmsv_waterlevelercore.dll out/gmsv_waterlevelercore_win64.dll
#   cargo build --release --target x86_64-pc-windows-gnu --no-default-features --features client
#   cp target/x86_64-pc-windows-gnu/release/gmsv_waterlevelercore.dll out/gmcl_waterlevelercore_win64.dll
set -e

# Binary modules are only loaded from garrysmod/lua/bin (not from an addon's lua/bin),
# so the DLL goes there and the Lua addon (addon/waterleveler) goes to garrysmod/addons.
# Override the garrysmod folder with GMOD_DIR.
GMOD_DIR_DEFAULT="/c/Program Files (x86)/Steam/steamapps/common/GarrysMod/garrysmod"
ADDON="addon/waterleveler"

cd "$(dirname "${0}")"
mkdir -p out

# Tuned for this machine's CPU; the DLL may not run on older CPUs.
RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }-C target-cpu=native"
export RUSTFLAGS

case "$(uname -s)" in
    Linux)
        cargo build --release --no-default-features --features server
        cp "target/release/libgmsv_waterlevelercore.so" "out/gmsv_waterlevelercore_linux64.dll"
        echo "built out/gmsv_waterlevelercore_linux64.dll (put it in garrysmod/lua/bin)"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        cargo build --release --no-default-features --features server
        cp "target/release/gmsv_waterlevelercore.dll" "out/gmsv_waterlevelercore_win64.dll"
        cargo build --release --no-default-features --features client
        cp "target/release/gmsv_waterlevelercore.dll" "out/gmcl_waterlevelercore_win64.dll"
        echo "built out/gmsv_waterlevelercore_win64.dll and out/gmcl_waterlevelercore_win64.dll"
        GMOD="${GMOD_DIR:-${GMOD_DIR_DEFAULT}}"
        mkdir -p "${GMOD}/lua/bin" "${GMOD}/addons/waterleveler"
        cp "out/gmsv_waterlevelercore_win64.dll" "${GMOD}/lua/bin/gmsv_waterlevelercore_win64.dll"
        cp "out/gmcl_waterlevelercore_win64.dll" "${GMOD}/lua/bin/gmcl_waterlevelercore_win64.dll"
        cp -r "${ADDON}/." "${GMOD}/addons/waterleveler/"
        echo "server and client modules -> ${GMOD}/lua/bin, addon -> ${GMOD}/addons/waterleveler"
        ;;
    *)
        echo "unsupported host: $(uname -s)" >&2
        exit 1
        ;;
esac
