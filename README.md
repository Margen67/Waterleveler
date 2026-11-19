# waterleveler (gmsv_waterlevelercore)

Per-entity water level overrides for Garry's Mod, as a dependency-free Rust
binary module. 64-bit (x86-64 branch) on Windows and Linux. Server realm only.

## Install

GMod only loads binary modules from `garrysmod/lua/bin/` (not from an addon's
`lua/bin`), so install in two parts: the DLL into `garrysmod/lua/bin/`, and the
`addon/waterleveler` folder (Lua library + volume script) into `garrysmod/addons/`.
`build.sh` does both on Windows (it installs the server and the client module). The CI artifact has both too. Files:

| Platform | File name                       |
|----------|---------------------------------|
| Linux 64 | `gmsv_waterlevelercore_linux64.dll`   |
| Win 64   | `gmsv_waterlevelercore_win64.dll`     |


(The Linux file really is named `.dll`; GMod's module loader expects that.)

```lua
require("waterleveler")   -- Lua library (lua/includes/modules/waterleveler.lua);
                        -- it requires the binary module "waterlevelercore" itself
```

The library exposes every function of the binary module under `waterleveler.*`
(the raw table is also `waterleveler.Core` / the global `waterlevelercore`) and adds
`waterleveler.IsAvailable()`, `waterleveler.AddBox(cornerA, cornerB)` and
`waterleveler.InWater(pos)`.

```lua
-- size is a Vector or a number (cube); anchor: "center" (default), "bottom", "mins"
local id, err = waterleveler.CreateVolume(Vector(0, 0, 100), Vector(512, 512, 256))
local id2 = waterleveler.CreateVolume(tr.HitPos, 512, "bottom")   -- sits on the ground
waterleveler.RemoveVolume(id)
```

## Lua API

```lua
waterleveler.Set(ent, level)   -- level: integer 0..3.  true | nil, err
waterleveler.Get(ent)          -- number | nil
waterleveler.Clear(ent)        -- bool (was there an override)
waterleveler.ClearAll()
waterleveler.IsHooked()        -- engine hook active?
waterleveler.HookStatus()      -- e.g. "hooked CBaseEntity::SetWaterLevel at server.dll+<rva>"
waterleveler.HookHits()        -- engine calls whose value was replaced
waterleveler.SetSaveKey(key)   -- datamap key written each tick; "" disables that layer

ent:SetWaterLevelOverride(level)
ent:GetWaterLevelOverride()
ent:ClearWaterLevelOverride()
```

Levels follow `Entity:WaterLevel()`: 0 none, 1 feet, 2 waist, 3 submerged.
Overrides are dropped when the entity is removed and when the module unloads.

## How it works

1. **Engine hook (Windows x64 only).** `CBaseEntity::SetWaterLevel(int)` is
   detoured. It is the engine's single writer of `m_nWaterLevel` (18 call
   sites), so when an entity has an override the engine itself uses it, and
   networking and `GM:OnEntityWaterLevelChanged` stay consistent. The function
   is found at load time by a 63-byte signature (must match exactly once) in
   the executable sections of `server.dll`. If it does not match, for example
   on a different build, the hook is skipped and the other layers still work.
2. **`Entity:WaterLevel()` wrapper.** Lua always sees the override.
3. **Per-tick datamap write.** `Entity:SetSaveValue("m_nWaterLevel", level)`
   covers code that writes the byte directly (one player-state restore path
   does) and is the only engine-side layer on Linux.

Facts taken from the 64-bit `server.dll`: `m_nWaterLevel` is a
`FIELD_CHARACTER` datamap field at `this+0x2ab` (SAVE|KEY, map key
`waterlevel`); the entity index is `*(i16*)(*(void**)(this+0x30)+6)`
(`m_pPev->m_EdictIndex`, the same thing `Entity:EntIndex` reads).

## Water volumes (Windows x64)

```lua
local id = waterleveler.AddVolume(x1, y1, z1, x2, y2, z2 [, liquid])  -- world-space box; liquid "water" (default) or "slime"
waterleveler.RemoveVolume(id)
waterleveler.ClearVolumes()
waterleveler.VolumeCount()
waterleveler.PointInVolume(x, y, z)   -- pure lookup
waterleveler.IsContentsHooked()
waterleveler.ContentsHits()           -- engine point queries answered with water
waterleveler.TraceHits()              -- traces rewritten or flagged by a volume
waterleveler.BubbleClips()            -- bullet bubble trails clipped to a volume
waterleveler.AttachFluid(physobj, nx, ny, nz, dist [, damping] [, options])  -- experimental buoyancy; options.contents = "water"|"slime"
waterleveler.DetachFluid(id)
waterleveler.FluidCount()
```

Every point inside a volume reports `CONTENTS_WATER` to the engine. This works
by replacing vtable slot 0 of the engine's `IEngineTrace` object
(`GetPointContents`); no code is patched. The object is found through the
`enginetrace` pointer inside `CBaseEntity::UpdateWaterState` (located by
signature), which is the exact object server code uses. `UpdateWaterState`
tests the result against `MASK_WATER` (0x4030) at the feet, waist and eyes to
set water level 1, 2 and 3, so volumes behave like map water for everything
that goes through it: NPCs, `Entity:WaterLevel()`, `OnEntityWaterLevelChanged`
and server-side player movement.

Limits (read these):
- **No real water surface.** Rendering water needs map data compiled into the
  BSP; this cannot create it. `addon/waterleveler/lua/autorun/waterleveler_volumes.lua` draws a translucent
  box client-side as a placeholder.
- **Player prediction.** The client predicts swimming with its own trace object. With the client module it knows the volumes too (see below); without it a player inside one may see prediction corrections (stutter).
- **Buoyancy.** Props float only if a vphysics
  fluid controller exists; point contents alone do nothing. A map's water brush
  is a static physics object with the `water` material plus
  `physenv->CreateFluidController(obj, &fluidparams_t)` (vtable slot 11; read
  from server.dll's world setup and vphysics.dll). `waterleveler.AttachFluid(
  physobj, nx, ny, nz, dist [, damping] [, options])` does the same for a PhysObj you
  created (plane in world space, normal out of the fluid) and
  `options` is an optional table (also accepted as the 6th argument) with `damping` (default 0.01), `torque`, `viscosity`, `aerodynamics` (boolean) and the flow `currentx`, `currenty`, `currentz` (units/s). The defaults are what a map's water brush has: the engine reads only the plane, current, damping and contents from the map, so torque, viscosity and aerodynamics are 0. The Water Volume tool has a control for each. The fluid object's physics material (its density sets how high props float) follows the liquid: `water` or `slime`. `waterleveler.DetachFluid(id)` removes it; **detach before the entity is
  removed**. The `physenv` pointer is found by signature. the Water Volume tool builds one. Unknowns: whether a Lua-created
  object is accepted exactly like the world's, and the engine-side teardown.
- **Traces.** `IEngineTrace::TraceRay` (vtable slot 4, 214 call sites in
  server.dll) is hooked too: when the mask contains `CONTENTS_WATER`, a ray or
  hull that enters a volume from outside before it hits anything else is
  rewritten as a water-surface hit (fraction, endpos, plane, contents). This is
  what the engine's splash code (`ImpactWater`, bullet splash) uses to find the
  surface. `m_pEnt` is left as the engine set it, and `ClipRayToEntity`/`ClipRayToCollideable` are not
  hooked. `waterleveler.TraceHits()` counts rewrites.
  A ray that **starts inside** a volume behaves like one starting inside a
  real water brush: `startsolid`, and when it also ends inside, `allsolid`
  with fraction 0 and the end point at the start. The engine's water-height
  probe (a downward `MASK_WATER` ray from 256 units above the entity) relies on that; before this, a probe starting inside
  a volume looked like "no water" and the surface height collapsed to the
  entity's own height.
- **Bullet bubble trails.** `UTIL_BubbleTrail` clips a
  trail by water depth only, never sideways, and sends one temp entity with
  `count` bubbles for the whole remaining segment. A shot leaving a volume
  through a side wall therefore produced a trail as long as its flight, and a
  few of those overflow the client's 500 temp-ent limit ("Overflow 500
  temporary ents!"). The function is detoured: the trail is clipped to the
  volume it is in (or enters) and `count` is scaled by the kept fraction.
  Trails outside every volume are untouched.
- **Client-decided effects** (the flat `effects/splashwake1` ripple, `FX_WaterRipple`) are
  only created if the client's *own* `TraceRay` with `MASK_WATER` hits something just below
  the splash point. The splash effect data the server sends is enough for the particle
  splashes, but the ripples need the client module (`gmcl_waterlevelercore_win64.dll`), which
  hooks the client's trace so the engine makes them itself. Without it there are no ripples.

Lua is linted and formatted with [GLuaFixer](https://github.com/FPtje/GLuaFixer) (`glualint.json`; CI checks both; `glualint --pretty-print-files addon` formats).

Server and client script: `addon/waterleveler/lua/autorun/waterleveler_volumes.lua` (loaded automatically as part of the addon).
Tool: **Water Volume** under Tools > Construction (size per axis, vertical offset up to the
biggest size, buoyancy and material; an outline previews the volume where you
are looking).
Material icons in the spawnmenu (the stock one and Extended Spawnmenu's) have a right-click option "Use with Water Volume tool" that sets `waterleveler_material` and switches to the tool.

Every numeric `waterleveler_*` convar has limits (sizes 1-32768, offset +-32768, damping/torque/viscosity 0-10, current +-1000), and the server clamps what it receives. The muffled underwater explosion sound needs the blast to be more than 128 units below the surface (the client looks up to 512).

Debugging commands (the volumes themselves come from the Water Volume tool):
`waterleveler_status` (server: hook status, counters, your water level),
`waterleveler_explode` (server, superadmin: explode where you look, through the normal explosion path),
`waterleveler_client_status` (client: module status, counters, your eyes against the volumes) and
`waterleveler_watch [0|1]` (client: once a second, how many point queries and traces the engine made against the volumes, plus what `util.PointContents` answers at your eyes; for the underwater sound muffling).

## Engine layout notes

`Ray_t` differs between builds: the x86-64 branch inserted an 8-byte member at
+0x40, so `m_IsSwept` moved from +0x41 to +0x49. The module reads the offset
from the engine's own ray-building helper at load time (shown in
`HookStatus()` as `Ray_t m_IsSwept at +0x..`).

## Verified vs not

Verified (`cargo test`, with `SERVER_DLL_X64` pointing at your `server.dll`):
the signature matches exactly once; the PE section scanner finds it in a
loader-style mapped image; the detour + trampoline run the **real
`SetWaterLevel` machine code** from that DLL, substitute the override,
pass non-overridden calls through, and uninstall restores the bytes exactly.

Not verified, because it needs the actual game:
- Loading into a live Garry's Mod, the `lua_shared` symbol lookup, and the Lua
  glue.
- The Windows-specific glue (`VirtualAlloc/VirtualProtect/GetModuleHandleA`).
  It is a few dozen lines and could not be compiled here (no Windows target).
- Whether `SetSaveValue` accepts a number for a `FIELD_CHARACTER` field. If
  not, use `waterleveler.SetSaveKey("")`; the engine hook does not depend on it.
- Linux: no engine hook (a Linux `server.so` is a different binary).
- Patching happens at module load. Do it while the server is idle (normal
  `require` at startup is fine).

## Build

`./build.sh` (Linux host -> linux64, MSYS2/Git Bash -> win64). No crates are
fetched. Cross-compiling Windows from Linux is described at the top of
`build.sh`. Needs Rust 1.77+.

Tests: `SERVER_DLL_X64=/path/to/server.dll cargo test --release`

## Removing a volume with props in it

vphysics keeps a list of the fluids each physics object touches and calls the game's `FluidEndTouch` for them when the object is destroyed, even for a fluid controller that was destroyed in the meantime. The splash code in there read the dead controller and crashed the game. The function (`CCollisionEvent::FluidEndTouch`, found by signature) is detoured: for a controller this module destroyed, it skips the splash and does the rest (clears `EFL_TOUCHING_FLUID`, sends `ENTITY_EVENT_WATER_UNTOUCH`). `HookStatus()` shows `FluidEndTouch` when it is active.

## Explosions

Explosive entities pick the water explosion (underwater sound, splash and wake, all produced by the client from one effect) from their own water level, not from point contents; one of them needs a water level of 2 or more. Three of those server functions are patched so an entity whose origin is inside a volume has its water level raised to 2 before the original runs (`waterleveler.ExplosionFixes()` counts them; `HookStatus()` reports how many of the 3 signatures matched).

## Trace hits and the hit entity

A water hit produced for a volume names the world entity (entry 0 of the engine's entity array, found by signature) as the hit entity, like a real water brush. Without it, code that calls a virtual on the trace's entity (for example the SLAM's impact handling) crashed. If the world entity cannot be found, hits the engine gave no entity are skipped.

## Explosion events

Barrels, RPGs, grenades and SLAMs send the client an explosion *event*; the client picks the normal or the water explosion (underwater sound, splash, wake) from its own point-contents check, which a volume cannot feed. `CBaseTempEntity::Create` (every temp entity goes through it) is therefore detoured and checks for the `CTEExplosion` vtable (found through its RTTI name): an explosion whose origin is inside a volume is sent as the `WaterSurfaceExplosion` effect instead (the same effect and data the engine's own water-explosion code sends) and the event is dropped. The effect is sent at the blast point; the client traces up from it with `MASK_WATER` to find the surface (splash and wake go there) and the depth, and plays the muffled `WaterExplosionEffect.Sound` file when that depth is over 128 units. The client module's trace hook therefore reports the exit point (`fractionleftsolid`) for rays that start inside a volume. `waterleveler.ExplosionsRedirected()` counts them.

## Water or slime

`AddVolume`, `AddBox` and `CreateVolume` take an optional liquid, `"water"` (default) or `"slime"`, and
`AttachFluid` takes `contents = "water"|"slime"` in its options (keep it equal to the volume's liquid). The
engine derives the water type from the point contents, so slime volumes get slime-coloured splashes and
ripples and a different swim sound value, players cannot wade in them, and underwater explosions do not
use the water explosion effect (the engine only does that for water). The Water Volume tool has a Liquid
dropdown, and the liquid also picks the fluid object's physics material (`water` or `slime`).

## License

Copyright (C) 2026 Margen67. AGPL-3.0-or-later; see `LICENSE` (the full GNU AGPL v3 text).
