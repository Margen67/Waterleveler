#!/bin/sh
# Builds the 64-bit Linux modules (the 64-bit server, server_srv.so, and the 64-bit client,
# client.so; two builds) and renames them the way Garry's Mod expects:
#   gmsv_waterlevelercore_linux64.dll and gmcl_waterlevelercore_linux64.dll
# For Windows use build.ps1.
set -e

# Binary modules are only loaded from garrysmod/lua/bin (not from an addon's lua/bin),
# so the DLL goes there and the Lua addon (addon/waterleveler) goes to garrysmod/addons
# (an existing copy is deleted first).
# Override the garrysmod folder with GMOD_DIR. The default is the dedicated server's
# (GarrysModDS).
GMOD_DIR_DEFAULT="${HOME}/.local/share/Steam/steamapps/common/GarrysModDS/garrysmod"
# Used when the dedicated server's folder (GarrysModDS) does not exist.
GMOD_DIR_FALLBACK="${HOME}/.local/share/Steam/steamapps/common/GarrysMod/garrysmod"
ADDON="addon/waterleveler"

if [ "$(uname -s)" != "Linux" ]; then
    echo "build.sh is for Linux; use build.ps1 on Windows" >&2
    exit 1
fi

cd "$(dirname "${0}")"
mkdir -p out

# Tuned for this machine's CPU; the DLL may not run on older CPUs.
RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }-C target-cpu=native"
export RUSTFLAGS

cargo build --release --no-default-features --features server
cp "target/release/libgmsv_waterlevelercore.so" "out/gmsv_waterlevelercore_linux64.dll"
cargo build --release --no-default-features --features client
cp "target/release/libgmsv_waterlevelercore.so" "out/gmcl_waterlevelercore_linux64.dll"
echo "built out/gmsv_waterlevelercore_linux64.dll and out/gmcl_waterlevelercore_linux64.dll"

GMOD="${GMOD_DIR:-${GMOD_DIR_DEFAULT}}"
if [ -z "${GMOD_DIR}" ] && [ ! -d "${GMOD}" ] && [ -d "${GMOD_DIR_FALLBACK}" ]; then
    echo "${GMOD} not found, using ${GMOD_DIR_FALLBACK}"
    GMOD="${GMOD_DIR_FALLBACK}"
fi
# Delete the old copy first, so files removed from the addon don't linger.
rm -rf "${GMOD}/addons/waterleveler"
mkdir -p "${GMOD}/lua/bin" "${GMOD}/addons/waterleveler"
cp "out/gmsv_waterlevelercore_linux64.dll" "${GMOD}/lua/bin/gmsv_waterlevelercore_linux64.dll"
cp "out/gmcl_waterlevelercore_linux64.dll" "${GMOD}/lua/bin/gmcl_waterlevelercore_linux64.dll"
cp -r "${ADDON}/." "${GMOD}/addons/waterleveler/"
echo "server and client modules -> ${GMOD}/lua/bin, addon -> ${GMOD}/addons/waterleveler"
