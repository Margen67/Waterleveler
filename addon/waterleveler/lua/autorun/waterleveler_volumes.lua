-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- waterleveler_volumes.lua (shared)
-- The part of the script both realms need: the WLWater table and WLWater.Placement. The
-- server half is autorun/server/waterleveler_volumes.lua and the client half
-- (drawing) autorun/client/waterleveler_volumes.lua.
--
-- Volumes are created with the Water Volume tool (Tools > Construction) or
-- the waterleveler library. Debugging commands:
--   waterleveler_status         (server) hook status, counters, your water level
--   waterleveler_probe          (server) where your feet, waist and eyes are against the nearest
--                               volume (its box, its surface) and your water level
--   waterleveler_explode        (server) explode where you look, through the normal
--                               explosion path (water explosion, sound, particles)
--   waterleveler_client_status  (client) client hook status, counters, where your eyes are
--   waterleveler_watch [0|1]    (client) print, once a second, what the client's water
--                               queries answer at your eyes (underwater muffling)
--
-- The volume is REAL to the engine's water logic (swimming, WaterLevel(),
-- OnEntityWaterLevelChanged) because the module makes the engine report
-- CONTENTS_WATER for every point inside it. The translucent box you see is
-- only a client-side placeholder: there is no real water surface shader.
if SERVER then AddCSLuaFile(); end
WLWater = WLWater or {};
--- Where a volume of `size` (Vector) goes for the trace `tr`: it rests against the surface the
--- trace hit (floor, wall or ceiling), then moves `offset` units up (down when negative).
--- Returns mins, maxs. Shared by the tool's preview (client) and the spawn (server).
function WLWater.Placement(tr, size, offset)
    local half = size * 0.5;
    local n = tr.HitNormal;
    -- A trace stops DIST_EPSILON (1/32) short of the surface it hit, on the open side. A volume
    -- starting exactly there would put a bullet's impact point (the same 1/32 off the surface)
    -- on its very edge, where rounding decides whether it is "in the water" and so whether
    -- there is a splash; it would also leave a gap under the player's feet. The volume is
    -- pushed back into the surface instead, so it starts at the surface itself, like a brush.
    local center = tr.HitPos + Vector(n.x * half.x, n.y * half.y, n.z * half.z) - n * 0.0625;
    center.z = center.z + (offset or 0);
    return center - half, center + half, center;
end
