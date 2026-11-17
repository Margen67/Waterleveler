-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- waterleveler: Lua library on top of the waterlevelercore binary module.
--   require("waterleveler")
-- Every function of the binary module is available unchanged (waterleveler.Set,
-- waterleveler.AddVolume, ...); the helpers below only add Vector-based
-- convenience on top. Server realm only.
if not SERVER then return end

require("waterlevelercore")

local core = waterlevelercore
if not core then
    ErrorNoHalt("[waterleveler] waterlevelercore loaded but did not provide its table\n")
    return
end

waterleveler = waterleveler or {}
local lib = waterleveler

for k, v in pairs(core) do
    lib[k] = v
end

--- Raw module table, for anything the wrappers do not cover.
lib.Core = core

--- true when the engine hooks are installed (volumes affect contents/traces).
function lib.IsAvailable()
    return core.IsContentsHooked() == true
end

--- Add a water volume from two corner vectors (any order). Returns id | nil, err.
function lib.AddBox(a, b)
    local mins = Vector(math.min(a.x, b.x), math.min(a.y, b.y), math.min(a.z, b.z))
    local maxs = Vector(math.max(a.x, b.x), math.max(a.y, b.y), math.max(a.z, b.z))
    return core.AddVolume(mins.x, mins.y, mins.z, maxs.x, maxs.y, maxs.z)
end

--- Create a water volume from a position and a size.
---   pos    Vector, world position (see `anchor`)
---   size   Vector (x, y, z extents) or a number for a cube
---   anchor where `pos` sits on the box: "center" (default), "bottom" (centre of the
---          bottom face, so the surface is pos.z + size.z) or "mins" (lowest corner)
--- Returns the volume id, or nil, err. Remove it with waterleveler.RemoveVolume(id).
function lib.CreateVolume(pos, size, anchor)
    if type(size) == "number" then size = Vector(size, size, size) end
    if not isvector(pos) or not isvector(size) then
        return nil, "CreateVolume needs a position Vector and a size (Vector or number)"
    end
    if size.x <= 0 or size.y <= 0 or size.z <= 0 then
        return nil, "CreateVolume size must be positive"
    end
    anchor = anchor or "center"
    local mins
    if anchor == "center" then
        mins = pos - size * 0.5
    elseif anchor == "bottom" then
        mins = Vector(pos.x - size.x * 0.5, pos.y - size.y * 0.5, pos.z)
    elseif anchor == "mins" then
        mins = Vector(pos)
    else
        return nil, "CreateVolume anchor must be \"center\", \"bottom\" or \"mins\""
    end
    return core.AddVolume(mins.x, mins.y, mins.z, mins.x + size.x, mins.y + size.y, mins.z + size.z)
end

--- Is the world position inside any volume?
function lib.InWater(pos)
    return core.PointInVolume(pos.x, pos.y, pos.z)
end

return lib

-- Underwater explosions inside a volume: the effect's splash is on the surface, so the
-- module sends it without sound and queues the sound; it is played where the blast was.
hook.Add("Think", "waterleveler_explosion_sounds", function()
    while true do
        local x, y, z, scale = core.PopExplosionSound()
        if not x then return end
        sound.Play(scale > 128 and "WaterExplosionEffect.Sound" or "BaseExplosionEffect.Sound", Vector(x, y, z), 140, 100, 1)
    end
end)
