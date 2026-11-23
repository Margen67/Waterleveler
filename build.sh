#!/bin/sh
# Builds the 64-bit module for the current host and renames it the way Garry's Mod expects.
#   Linux  -> gmsv_waterlevelercore_linux64.dll (the 64-bit server, server_srv.so)
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
# so the DLL goes there and the Lua addon (addon/waterleveler) goes to garrysmod/addons
# (an existing copy is deleted first).
# Override the garrysmod folder with GMOD_DIR. The defaults are the Windows client's folder
# and, on Linux, the dedicated server's (GarrysModDS).
GMOD_DIR_DEFAULT="/c/Program Files (x86)/Steam/steamapps/common/GarrysMod/garrysmod"
GMOD_DIR_DEFAULT_LINUX="${HOME}/.local/share/Steam/steamapps/common/GarrysModDS/garrysmod"
# Used on Linux when the dedicated server's folder (GarrysModDS) does not exist.
GMOD_DIR_FALLBACK_LINUX="${HOME}/.local/share/Steam/steamapps/common/GarrysMod/garrysmod"
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
        echo "built out/gmsv_waterlevelercore_linux64.dll"
        GMOD="${GMOD_DIR:-${GMOD_DIR_DEFAULT_LINUX}}"
        if [ -z "${GMOD_DIR}" ] && [ ! -d "${GMOD}" ] && [ -d "${GMOD_DIR_FALLBACK_LINUX}" ]; then
            echo "${GMOD} not found, using ${GMOD_DIR_FALLBACK_LINUX}"
            GMOD="${GMOD_DIR_FALLBACK_LINUX}"
        fi
        # Delete the old copy first, so files removed from the addon don't linger.
        rm -rf "${GMOD}/addons/waterleveler"
        mkdir -p "${GMOD}/lua/bin" "${GMOD}/addons/waterleveler"
        cp "out/gmsv_waterlevelercore_linux64.dll" "${GMOD}/lua/bin/gmsv_waterlevelercore_linux64.dll"
        cp -r "${ADDON}/." "${GMOD}/addons/waterleveler/"
        echo "server module -> ${GMOD}/lua/bin, addon -> ${GMOD}/addons/waterleveler"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        cargo build --release --no-default-features --features server
        cp "target/release/gmsv_waterlevelercore.dll" "out/gmsv_waterlevelercore_win64.dll"
        cargo build --release --no-default-features --features client
        cp "target/release/gmsv_waterlevelercore.dll" "out/gmcl_waterlevelercore_win64.dll"
        echo "built out/gmsv_waterlevelercore_win64.dll and out/gmcl_waterlevelercore_win64.dll"
        GMOD="${GMOD_DIR:-${GMOD_DIR_DEFAULT}}"
        # Delete the old copy first, so files removed from the addon don't linger.
        rm -rf "${GMOD}/addons/waterleveler"
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
