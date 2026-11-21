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
--   waterleveler_debug [0|1]    (server) print every water trace near a volume and every
--                               point query just outside one, as the hooks answered them
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
--- How far the collision box of a volume placed by `WLWater.Placement` reaches into the surface
--- it rests on. The client draws the bottom of the volume this much shorter. The engine tests
--- for water 0.1 below where a bullet hit the floor (1/32 above it), so this must exceed 0.13.
WLWater.ContactDepth = 0.25;
--- Where a volume of `size` (Vector) goes for the trace `tr`: it rests against the surface the
--- trace hit (floor, wall or ceiling), then moves `offset` units up (down when negative).
--- Returns mins, maxs, center. Shared by the tool's preview (client) and the spawn (server).
function WLWater.Placement(tr, size, offset)
    local half = size * 0.5;
    local n = tr.HitNormal;
    local center = tr.HitPos + Vector(n.x * half.x, n.y * half.y, n.z * half.z);
    center.z = center.z + (offset or 0);
    local mins, maxs = center - half, center + half;
    -- A trace stops DIST_EPSILON (1/32) short of the surface it hit, on the open side, and a
    -- bullet's impact point is the same 1/32 off the surface. A box that began exactly at the
    -- trace's hit position would have that impact point on its very edge, where rounding
    -- decides whether it is "in the water" (and so whether there is a splash). The face that
    -- touches the surface reaches ContactDepth into it instead, like a brush.
    local depth = WLWater.ContactDepth;
    if n.x > 0.01 then
        mins.x = mins.x - depth;
    elseif n.x < -0.01 then
        maxs.x = maxs.x + depth;
    end

    if n.y > 0.01 then
        mins.y = mins.y - depth;
    elseif n.y < -0.01 then
        maxs.y = maxs.y + depth;
    end

    if n.z > 0.01 then
        mins.z = mins.z - depth;
    elseif n.z < -0.01 then
        maxs.z = maxs.z + depth;
    end
    return mins, maxs, center;
end
