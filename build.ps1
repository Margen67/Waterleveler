# Builds the 64-bit Windows modules (two builds: the server module has only the server code,
# the client module only the client code) and renames them the way Garry's Mod expects:
#   gmsv_waterlevelercore_win64.dll and gmcl_waterlevelercore_win64.dll
# For Linux use build.sh.
#
# Binary modules are only loaded from garrysmod/lua/bin (not from an addon's lua/bin),
# so the DLL goes there and the Lua addon (addon/waterleveler) goes to garrysmod/addons
# (an existing copy is deleted first).
# Override the garrysmod folder with -GmodDir or the GMOD_DIR environment variable.
param(
    [string]$GmodDir = $env:GMOD_DIR
)

$ErrorActionPreference = 'Stop'

if (!$GmodDir) {
    $GmodDir = "${env:ProgramFiles(x86)}\Steam\steamapps\common\GarrysMod\garrysmod"
}
$Addon = 'addon\waterleveler'

Set-Location -LiteralPath $PSScriptRoot
New-Item -ItemType Directory -Force -Path out | Out-Null

# Tuned for this machine's CPU; the DLL may not run on older CPUs. Any warning (from rustc or
# cargo itself) fails the build, as in CI.
$env:RUSTFLAGS = "$env:RUSTFLAGS -C target-cpu=native -D warnings".Trim()
$env:CARGO_BUILD_WARNINGS = 'deny'

function Invoke-ModuleBuild([string]$Feature, [string]$Output) {
    cargo build --release --no-default-features --features $Feature
    if ($LASTEXITCODE -ne 0) { throw "cargo build ($Feature) failed" }
    Copy-Item -Force 'target\release\gmsv_waterlevelercore.dll' "out\$Output"
}

Invoke-ModuleBuild 'server' 'gmsv_waterlevelercore_win64.dll'
Invoke-ModuleBuild 'client' 'gmcl_waterlevelercore_win64.dll'
Write-Output 'built out\gmsv_waterlevelercore_win64.dll and out\gmcl_waterlevelercore_win64.dll'

# Delete the old copy first, so files removed from the addon don't linger.
$AddonDest = Join-Path $GmodDir 'addons\waterleveler'
if (Test-Path -LiteralPath $AddonDest) { Remove-Item -Recurse -Force -LiteralPath $AddonDest }
New-Item -ItemType Directory -Force -Path (Join-Path $GmodDir 'lua\bin'), $AddonDest | Out-Null
Copy-Item -Force 'out\gmsv_waterlevelercore_win64.dll' (Join-Path $GmodDir 'lua\bin')
Copy-Item -Force 'out\gmcl_waterlevelercore_win64.dll' (Join-Path $GmodDir 'lua\bin')
Copy-Item -Recurse -Force "$Addon\*" $AddonDest
Write-Output "server and client modules -> $GmodDir\lua\bin, addon -> $AddonDest"
