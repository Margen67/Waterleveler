# waterleveler

Real, engine-level water volumes for Garry's Mod, made from Lua. A volume is not a trick
on top of the game: the engine itself treats it as water. It also lets you override the
water level of single entities.

It is a Rust binary module plus a Lua addon, for the default, dev and x86-64 branches of
the 64-bit game (Windows x64 and Linux x64). 32-bit Garry's Mod and macOS will never be
supported.

## What a volume does like real water

| Feature          | Details                                                                    |
|------------------|----------------------------------------------------------------------------|
| Swimming         | Players and NPCs swim, wade and drown; water level and its hooks work      |
| Splashes         | Bullets, grenades, props and players splash and make ripples               |
| Buoyancy         | Props float; damping, currents, torque and viscosity are adjustable        |
| Explosions       | Underwater explosions use the water effect, with the muffled sound         |
| Bubble trails    | Bullets leave bubbles, clipped to the volume                               |
| Water or slime   | Slime has its own splashes, slower wading and physics material             |
| Underwater view  | The screen warp and underwater sound muffling work                         |
| Multiplayer      | Clients that join later see every volume; undo and cleanup work            |

Details of how each one works are in [NOTES.md](NOTES.md).

## Install

1. Download the build (CI artifact) or build it yourself (see below).
2. Copy the module files into `garrysmod/lua/bin/`. Garry's Mod does not load binary modules from an addon's `lua/bin`.
3. Copy the `addon/waterleveler` folder into `garrysmod/addons/`.
4. Every player who should see volumes correctly needs the **client** module too, because the server cannot send binary modules. Without it players see prediction errors and miss the effects their client makes.

| Platform | Realm  | File name                           |
|----------|--------|-------------------------------------|
| Windows  | server | `gmsv_waterlevelercore_win64.dll`   |
| Windows  | client | `gmcl_waterlevelercore_win64.dll`   |
| Linux    | server | `gmsv_waterlevelercore_linux64.dll` |
| Linux    | client | `gmcl_waterlevelercore_linux64.dll` |

The Linux files really end in `.dll`; Garry's Mod's module loader expects that.

`build.sh` (Linux) and `build.ps1` (Windows) do steps 2 and 3. They delete the installed addon first.
On Linux the default target is the dedicated server (`~/.local/share/Steam/steamapps/common/GarrysModDS/garrysmod`, or the `GarrysMod` folder if that is missing). Set `GMOD_DIR` to use another.

## Water Volume tool

Tools tab > Construction > **Water Volume**.

| Control        | What it does                                                          |
|----------------|-----------------------------------------------------------------------|
| Left click     | Create a volume where you aim; it rests on the surface you aim at     |
| Reload         | Remove every volume that touches the preview                          |
| Size, Offset   | Size per axis (or all at once) and a vertical offset                  |
| Liquid         | Water or slime                                                        |
| Buoyancy       | Props float; damping, torque, viscosity, aerodynamics and currents    |
| Material       | Any material, or a preset; optionally `_beneath` on the inside        |
| Solid color    | Draw one translucent color instead of a material                      |
| Outline        | Draw the volume's edges in a color                                    |

Only superadmins can create volumes (anyone in singleplayer and for the listen server host).

## Lua API

Load the library with `require("waterleveler")`. It loads the binary module for you.

### Making volumes

Server side. This does everything the tool does: engine water, buoyancy, the box on every client, undo and cleanup.

```lua
-- Any two opposite corners. Returns the volume id, or nil, err.
local id, err = WLWater.Spawn(nil, mins, maxs)

-- ply (whose undo list and cleanup get it) and every option are optional:
local id = WLWater.Spawn(ply, mins, maxs, {
    liquid = "slime",                       -- default "water"
    fluid = false,                          -- default true: buoyancy for props
    material = "nature/water_movingplane",  -- default "": nothing is drawn
    beneath = false,                        -- default true: _beneath texture on the inside
    solid = true,                           -- default false: one color instead of the material
    color = Color(90, 190, 255, 110),       -- the solid color (default: blue or green)
    outline = true,                         -- default false: draw the edges
    outlineColor = Color(90, 190, 255),     -- default: the liquid's color
    fluidOptions = { damping = 0.05, currentx = 100 },
})

WLWater.RemoveBox(id)                       -- remove it everywhere
WLWater.RemoveInBox(mins, maxs)             -- remove every volume touching a box; returns how many
```

`WLWater.Placement(tr, size, offset)` (shared) returns the mins and maxs of a volume resting against what a trace hit.

The engine side only (no entity, no drawing, clients are not told):

```lua
local id = waterleveler.AddVolume(x1, y1, z1, x2, y2, z2 [, liquid])   -- "water" (default) or "slime"
waterleveler.RemoveVolume(id)
waterleveler.ClearVolumes()
waterleveler.PointInVolume(x, y, z)
```

`waterleveler.CreateVolume(center, size [, anchor])` takes a Vector or a number as the size, and an anchor of `"center"` (default), `"bottom"` or `"mins"`.

`fluidOptions` are the options of `waterleveler.AttachFluid(physobj, nx, ny, nz, dist [, damping] [, options])`:

| Option                         | Default | Notes                         |
|--------------------------------|---------|-------------------------------|
| `damping`                      | 0.01    | Drag                          |
| `torque`, `viscosity`          | 0       | Passed to vphysics unchanged  |
| `aerodynamics`                 | false   | Aerodynamic drag              |
| `currentx`, `currenty`, `currentz` | 0   | Flow in units per second      |

### Entity water level

```lua
ent:SetWaterLevelOverride(level)   -- 0 none, 1 feet, 2 waist, 3 submerged
ent:GetWaterLevelOverride()
ent:ClearWaterLevelOverride()

waterleveler.Set(ent, level)       -- the same, as functions
waterleveler.Get(ent)
waterleveler.Clear(ent)
waterleveler.ClearAll()
```

Overrides are dropped when the entity is removed and when the module unloads.

### Status

| Function                       | Returns                                         |
|--------------------------------|-------------------------------------------------|
| `waterleveler.IsAvailable()`   | Whether the module loaded                       |
| `waterleveler.IsHooked()`      | Whether the engine hook is active               |
| `waterleveler.HookStatus()`    | Which hooks are active and where                |

Console commands: `waterleveler_status` (server) and `waterleveler_client_status` (client). More are in [NOTES.md](NOTES.md).

## Original game bugs

These happen without this module too:

1. Crowbar hit splashes are missing in multiplayer.
2. Bullet bubble trails are missing in multiplayer.

## Build

1. Install Rust nightly (the `gmod` crate needs it).
2. Run `./build.sh` on Linux or `./build.ps1` in PowerShell on Windows.

The server and the client module are separate builds of one crate, selected with the `server` and `client` cargo features. Release builds enable one each, so each module only has the code it needs.

Tests need the game's binaries, which are not in the repo. Each environment variable points at one file; tests for a binary you do not set are skipped.

| Variable                   | File                                               |
|----------------------------|----------------------------------------------------|
| `SERVER_DLL_X64`           | Windows `server.dll`                               |
| `SERVER_SO_X64`, `CLIENT_SO_X64`       | Linux dev branch `server_srv.so`, `client.so` |
| `SERVER_SO_X86_64`, `CLIENT_SO_X86_64` | Linux x86-64 branch `server.so`, `client.so`  |
| `SERVER_SO_X86_64_CLIENT`  | The x86-64 branch game client's `server_client.so` |

```sh
SERVER_DLL_X64=/path/to/server.dll cargo test --release
```

Lua is linted and formatted with [GLuaFixer](https://github.com/FPtje/GLuaFixer) (`glualint --pretty-print-files addon` formats) and type-checked with [LuaLS](https://github.com/LuaLS/lua-language-server).

## More

[NOTES.md](NOTES.md): how it works, limits, what was verified, and debugging commands.

## License

Copyright (C) 2026 Margen67. AGPL-3.0-or-later; see `LICENSE`.
