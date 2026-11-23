# waterleveler (gmsv_waterlevelercore)

Per-entity water level overrides and fake water volumes for Garry's Mod, as a
dependency-free Rust binary module for the 64-bit (x86-64) branch. The server module
(`gmsv_`) and the client module (`gmcl_`) are separate builds. The water volumes work on the
64-bit Windows server and client and on the 64-bit Linux server (`server_srv.so`); the Linux
client has no engine hooks, only the water level datamap layer.

## Install

GMod only loads binary modules from `garrysmod/lua/bin/` (not from an addon's
`lua/bin`), so install in two parts: the DLL into `garrysmod/lua/bin/`, and the
`addon/waterleveler` folder (Lua library + volume script) into `garrysmod/addons/`.
`build.sh` does both on Windows and Linux (the Linux default is the dedicated server, `~/.local/share/Steam/steamapps/common/GarrysModDS/garrysmod`; falls back to `GarrysMod/garrysmod` there if that folder is missing; override with `GMOD_DIR`) (it deletes the installed addon first, then copies the server
and the client module and the addon). The CI artifact has everything too. Files:

| Platform | Realm  | File name                           |
|----------|--------|-------------------------------------|
| Linux 64 | server | `gmsv_waterlevelercore_linux64.dll` |
| Win 64   | server | `gmsv_waterlevelercore_win64.dll`   |
| Win 64   | client | `gmcl_waterlevelercore_win64.dll`   |


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
waterleveler.HookStatus()      -- e.g. "hooked CBaseEntity::SetWaterLevel at server module+<rva>"
waterleveler.HookHits()        -- engine calls whose value was replaced
waterleveler.SetSaveKey(key)   -- datamap key written each tick; "" disables that layer

ent:SetWaterLevelOverride(level)
ent:GetWaterLevelOverride()
ent:ClearWaterLevelOverride()
```

Levels follow `Entity:WaterLevel()`: 0 none, 1 feet, 2 waist, 3 submerged.
Overrides are dropped when the entity is removed and when the module unloads.

## Making volumes from a script

Two levels, both server side:

```lua
-- Everything the Water Volume tool does: engine water, buoyancy, the translucent box on every
-- client, clients that join later, undo and cleanup. Returns the volume id, or nil, err.
local id, err = WLWater.Spawn(nil, mins, maxs)   -- any two opposite corners

-- ply (whose undo list and cleanup get it) and every option are optional:
local id = WLWater.Spawn(ply, mins, maxs, {
    liquid = "slime",          -- default "water"
    fluid = false,             -- default true: buoyancy for props
    material = "nature/water_movingplane",  -- default "": nothing is drawn (no mesh is made)
    beneath = false,           -- default true: the inside shows the material's _beneath texture
    fluidOptions = { damping = 0.05, currentx = 100 },   -- the options of waterleveler.AttachFluid;
                               -- default {}: a map water brush's values (damping 0.01, no torque,
                               -- viscosity, aerodynamics or current)
})
WLWater.RemoveBox(id)          -- removes it from the module and the clients

-- Only the engine side (point contents, traces, swimming, splashes). Clients are not told:
-- nothing is drawn and a player's client-side prediction does not know the volume.
local id = waterleveler.AddVolume(x1, y1, z1, x2, y2, z2)   -- liquid optional, default "water"
```

`WLWater.Placement(tr, size, offset)` (shared) gives the mins, maxs and center for a volume that
rests against what a trace hit. Volumes are cleared on map cleanup and when the module unloads.
Needs the Windows x64 server or client, or the Linux x64 server: elsewhere (including the Linux client) the module loads but volumes do nothing.

## How it works

1. **Engine hook (Windows x64 and Linux x64 servers).** `CBaseEntity::SetWaterLevel(int)` is
   detoured. It is the engine's single writer of `m_nWaterLevel` (18 call
   sites), so when an entity has an override the engine itself uses it, and
   networking and `GM:OnEntityWaterLevelChanged` stay consistent. The function
   is found at load time by a 63-byte signature (must match exactly once) in
   the executable sections of `server.dll` (`server_srv.so` on Linux, with its own signature). If it does not match, for example
   on a different build, the hook is skipped and the other layers still work.
2. **`Entity:WaterLevel()` wrapper.** Lua always sees the override.
3. **Per-tick datamap write.** `Entity:SetSaveValue("m_nWaterLevel", level)`
   covers code that writes the byte directly (one player-state restore path
   does).

Facts taken from the 64-bit Windows `server.dll` (the Linux server's layout is noted in `src/lib.rs`): `m_nWaterLevel` is a
`FIELD_CHARACTER` datamap field at `this+0x2ab` (SAVE|KEY, map key
`waterlevel`); the entity index is `*(i16*)(*(void**)(this+0x30)+6)`
(`m_pPev->m_EdictIndex`, the same thing `Entity:EntIndex` reads).

## Water volumes (Windows x64 and Linux x64 server)

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
waterleveler.ExplosionFixes()         -- explosives whose water level was raised to 2
waterleveler.ExplosionsRedirected()   -- explosion events sent as the water explosion
waterleveler.SetDebug(1)              -- log what the hooks answer near a volume ...
waterleveler.TakeDebugLog()           -- ... and read (and empty) that log; see waterleveler_debug
waterleveler.AttachFluid(physobj, nx, ny, nz, dist [, damping] [, options])  -- experimental buoyancy; options.contents = "water"|"slime"
waterleveler.DetachFluid(id)
waterleveler.FluidCount()
```

The client module (`gmcl_`) registers the same global `waterlevelercore` with the volume functions
(`AddVolume`, `RemoveVolume`, `ClearVolumes`, `VolumeCount`, `PointInVolume`), the counters
`IsContentsHooked`, `ContentsHits` and `TraceHits`, `SetDebug`, `TakeDebugLog` and `HookStatus`; the
override, fluid, bubble and explosion functions are server only.

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
  BSP; this cannot create it. `addon/waterleveler/lua/autorun/client/waterleveler_volumes.lua` draws a translucent
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
  surface. The hit entity is the world entity (see below), and `ClipRayToEntity`/`ClipRayToCollideable` are not
  hooked. `waterleveler.TraceHits()` counts rewrites.
  A ray that **starts inside** a volume behaves like one starting inside a
  real water brush: `startsolid`, and when it also ends inside, `allsolid`
  with fraction 0 and the end point at the start. The engine's water-height
  probe (a downward `MASK_WATER` ray from 256 units above the entity) relies on that; before this, a probe starting inside
  a volume looked like "no water" and the surface height collapsed to the
  entity's own height.
- **Bullet splashes.** The engine decides that a shot hit water from the point contents at the
  end of its normal trace, then re-traces with `CONTENTS_WATER` and tests the point 0.1 units
  *straight below* that trace's end. A real water brush only has a top face, so that point is
  always under the surface; a volume has side faces too. Three things make shots through the
  sides and next to the floor work: a point ray that enters a volume ends 1/8 unit inside it
  (hulls end on the face as before); the box a volume gets from `WLWater.Placement` reaches
  `WLWater.ContactDepth` (1/4 unit) into the surface it rests on, and the client draws the bottom
  that much shorter; and the shotgun's odd pellets, which are 3 unit hull traces whose end is the
  hull's centre, are answered for where the bullet's own line reaches the surface the hull
  stopped at. A bullet that lands on the floor outside a box, even by a fraction of a unit, gets
  no splash, as with real water.
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

Scripts (all loaded automatically as part of the addon), one file per realm so each side only gets the code it needs:
`autorun/waterleveler_volumes.lua` (shared: the `WLWater` table and placement),
`autorun/server/waterleveler_volumes.lua` (volume entities, undo and cleanup, networking) and
`autorun/client/waterleveler_volumes.lua` (materials and drawing), plus
`autorun/client/waterleveler.lua` (the client module loader and the spawnmenu option), all under `addon/waterleveler/lua/`.
Tool: **Water Volume** under Tools > Construction (size per axis, vertical offset up to the
biggest size, buoyancy and material; an outline previews the volume where you
are looking).
A volume whose material is empty is not drawn at all (no mesh, no client entity); it still works for the engine. The "Inside shows the material's _beneath texture" option (`waterleveler_beneath`, per volume) draws world water single-sided, with `<material>_beneath` on the inside; turned off, the material is drawn on both sides.

The "Underwater screen warp" option (`waterleveler_underwater_warp`, a client setting) turns off the `effects/water_warp01` screen overlay that the script draws while you are fully submerged in a volume.

Material icons in the spawnmenu (the stock one and Extended Spawnmenu's) have a right-click option "Use with Water Volume tool" that sets `waterleveler_material` and switches to the tool.

Every numeric `waterleveler_*` convar has limits (sizes 1-32768, offset +-32768, damping/torque/viscosity 0-10, current +-1000), and the server clamps what it receives. The muffled underwater explosion sound needs the blast to be more than 128 units below the surface (the client looks up to 512).

Debugging commands (the volumes themselves come from the Water Volume tool):
`waterleveler_status` (server: hook status, counters, your water level),
`waterleveler_probe` (server: your feet, waist and eyes against the nearest volume, and your water level),
`waterleveler_debug [0|1]` (server, superadmin: prints the status of both realms, then every water trace near a volume, every small hull trace (shotgun pellets) and every point query just outside one, as the hooks answered them; the status again when stopped),
`waterleveler_explode` (server, superadmin: explode where you look, through the normal explosion path),
`waterleveler_client_status` (client: module status, counters, your eyes against the volumes),
`waterleveler_envmap_strength` / `waterleveler_envmap_contrast` (client convars: reflection of `water_movingplane`-style world water),
`waterleveler_material_info [material]` (client: how a material is wrapped, its parameters and animation frames, and whether the wrapper's frame counters advance) and
`waterleveler_watch [0|1]` (client: once a second, how many point queries and traces the engine made against the volumes, plus what `util.PointContents` answers at your eyes; for the underwater sound muffling).

## Engine layout notes

`Ray_t` differs between builds: the x86-64 branch inserted an 8-byte member at
+0x40, so `m_IsSwept` moved from +0x41 to +0x49. The module reads the offset
from the engine's own ray-building helper at load time (shown in
`HookStatus()` as `Ray_t m_IsSwept at +0x..`).

## Verified vs not

Verified by `cargo test` (52 tests; the ones that need `server.dll` take its path in `SERVER_DLL_X64`, the Linux ones `server_srv.so` in `SERVER_SO_X64`):
the signatures match exactly once; the detour and trampoline run the **real
`SetWaterLevel` machine code** from that DLL, substitute the override, pass other calls
through, and uninstall restores the bytes exactly; the volume store and its lookup (lanes and
BVH), the trace maths (`plan_trace`: inside, leaving, entering, hulls, the entry depth), the
hull-pellet probe, and the vtable hooks run against fake `IEngineTrace` objects.

Verified in a live game (Windows x64, singleplayer and a dedicated server with a connected client, `gm_construct`): the
server and client modules load, the hooks install, volumes answer point queries and rewrite
traces, bullets (single and shotgun) splash on entering a volume through the top, the sides and
at the floor, props float up in a volume, removing a volume (with props inside it too) does not crash, the visible box has the collision box's size, and the water level, explosion and
bubble-trail patches act. The 64-bit Linux server is verified live too, in singleplayer (the client side is the unmodified Linux client): `CBaseEntity::SetWaterLevel`, point contents, trace rewriting, bubble trails, `FluidEndTouch`, all 3 explosion fixes and explosion events are active. CI builds each Windows module with `cargo clippy` without warnings.

On a dedicated server, a client that does **not** have the client module installed sees prediction
errors, and the effects the client makes from its own water queries are missing: the wake
splash, bullet and crowbar impact splashes, and the underwater muffling of explosions; a
glitchy underwater sound can play about every tick. With the client module installed
everything works. Binary modules cannot be sent by the server, so every player who should see
volumes correctly needs `gmcl_waterlevelercore_win64.dll` in `garrysmod/lua/bin`.

Known game bug (not caused by this module): crowbar hit splashes are missing in multiplayer.

Not verified:
- Listen servers, and a dedicated server with several clients at once.
- Buoyancy beyond the basics: `torque`, `viscosity`, `aerodynamics` and slime physics. They are
  passed to vphysics unchanged (as `torqueFactor`, `viscosityFactor` and `useAerodynamics`), and
  Valve has not documented what the first two do; a map's water brush has them at 0. `damping`
  is verified: at 10, props are stopped even when punted into the volume with the gravity gun.
  Currents (`currentx`, `currenty`, `currentz`) are verified too.
- Whether `SetSaveValue` accepts a number for a `FIELD_CHARACTER` field. If not, use
  `waterleveler.SetSaveKey("")`; the engine hook does not depend on it.
- Linux: only the 64-bit dedicated server (`server_srv.so`, or `server.so`) has hooks, found by
  signature, found and tested against the 64-bit dev branch build; other builds of it are untested. The Linux client has no engine hook, so no volumes there.
  The Linux server has no RTTI for `CTEExplosion`, so explosion events are recognised by the
  temp entity's name; entities are keyed by their handle's index instead of the edict's.
- Game updates: the hooks are found by signature, so an update that changes those functions
  disables the affected hook (`HookStatus()` and `waterleveler_status` say which) and leaves the others.
- Patching happens at module load. Do it while the server is idle (normal `require` at startup
  is fine).

## Build

`./build.sh` (Linux host -> linux64, MSYS2/Git Bash -> win64). No crates are
fetched. Cross-compiling Windows from Linux is described at the top of
`build.sh`. Needs Rust 1.77+.

The server and the client module are separate builds of the same crate: the `server`
and `client` cargo features (both on by default, which is what `cargo test` uses). The
release builds enable one each, so `gmsv_` has only the server code (overrides, fluids,
bubble and explosion patches) and `gmcl_` only the client code; the volume store and the
trace hooks are in both.

Tests: `SERVER_DLL_X64=/path/to/server.dll cargo test --release`, and for the Linux
signatures `SERVER_SO_X64=/path/to/server_srv.so cargo test --release`

## Removing a volume with props in it

vphysics keeps a list of the fluids each physics object touches and calls the game's `FluidEndTouch` for them when the object is destroyed, even for a fluid controller that was destroyed in the meantime. The splash code in there read the dead controller and crashed the game. The function (`CCollisionEvent::FluidEndTouch`, found by signature) is detoured: for a controller this module destroyed, it skips the splash and does the rest (clears `EFL_TOUCHING_FLUID`, sends `ENTITY_EVENT_WATER_UNTOUCH`). `HookStatus()` shows `FluidEndTouch` when it is active.

## Explosions

Explosive entities pick the water explosion (underwater sound, splash and wake, all produced by the client from one effect) from their own water level, not from point contents; one of them needs a water level of 2 or more. Three of those server functions are patched so an entity whose origin is inside a volume has its water level raised to 2 before the original runs (`waterleveler.ExplosionFixes()` counts them; `HookStatus()` reports how many of the 3 signatures matched).

## Trace hits and the hit entity

A water hit produced for a volume names the world entity (entry 0 of the engine's entity array, found by signature) as the hit entity, like a real water brush. Without it, code that calls a virtual on the trace's entity (for example the SLAM's impact handling) crashed. If the world entity cannot be found, hits the engine gave no entity are skipped.

## Explosion events

Barrels, RPGs, grenades and SLAMs send the client an explosion *event*; the client picks the normal or the water explosion (underwater sound, splash, wake) from its own point-contents check, which a volume cannot feed. `CBaseTempEntity::Create` (every temp entity goes through it) is therefore detoured and checks for the `CTEExplosion` vtable (found through its RTTI name; on Linux, which has no RTTI for it, the temp entity's name is checked instead): an explosion whose origin is inside a volume is sent as the `WaterSurfaceExplosion` effect instead (the same effect and data the engine's own water-explosion code sends) and the event is dropped. The effect is sent at the blast point; the client traces up from it with `MASK_WATER` to find the surface (splash and wake go there) and the depth, and plays the muffled `WaterExplosionEffect.Sound` file when that depth is over 128 units. The client module's trace hook therefore reports the exit point (`fractionleftsolid`) for rays that start inside a volume. `waterleveler.ExplosionsRedirected()` counts them.

## Water or slime

`AddVolume`, `AddBox` and `CreateVolume` take an optional liquid, `"water"` (default) or `"slime"`, and
`AttachFluid` takes `contents = "water"|"slime"` in its options (keep it equal to the volume's liquid). The
engine derives the water type from the point contents, so slime volumes get slime-coloured splashes and
ripples and a different swim sound value, players can wade in them, only more slowly, and underwater explosions do not
use the water explosion effect (the engine only does that for water). The Water Volume tool has a Liquid
dropdown, and the liquid also picks the fluid object's physics material (`water` or `slime`).

## License

Copyright (C) 2026 Margen67. AGPL-3.0-or-later; see `LICENSE` (the full GNU AGPL v3 text).
