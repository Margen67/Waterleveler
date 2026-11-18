-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- waterleveler_volumes.lua
-- Put this in garrysmod/lua/autorun/  (NOT autorun/server - it is shared, the
-- client half draws the placeholder box).
--
-- Console commands (server, superadmin or listen-server host):
--   wl_water_spawn [size] [nofluid]
--                           spawn a water volume (default 128x128x128, 1-16384) that
--                           rests on whatever surface you are looking at.
--                           Buoyancy is on by default; add "nofluid" to skip it.
--   wl_water_clear          remove all water volumes
--   wl_water_status         hook status, volume count, engine hit counter,
--                           and your own water level / whether you are inside
--
-- The volume is REAL to the engine's water logic (swimming, WaterLevel(),
-- OnEntityWaterLevelChanged) because the module makes the engine report
-- CONTENTS_WATER for every point inside it. The translucent box you see is
-- only a client-side placeholder: there is no real water surface shader.

local SIZE = 128
local MIN_SIZE, MAX_SIZE = 1, 16384
local DEFAULT_MATERIAL = "nature/water_movingplane" -- when a volume names none

WLWater = WLWater or {}

--- Where a volume of `size` (Vector) goes for the trace `tr`: it rests against the surface the
--- trace hit (floor, wall or ceiling), then moves `offset` units up (down when negative).
--- Returns mins, maxs. Shared by the tool's preview (client) and the spawn (server).
function WLWater.Placement(tr, size, offset)
    local half = size * 0.5
    local n = tr.HitNormal
    local center = tr.HitPos + Vector(n.x * half.x, n.y * half.y, n.z * half.z)
    center.z = center.z + (offset or 0)
    return center - half, center + half, center
end

if SERVER then
    AddCSLuaFile()
    require("waterleveler")
    util.AddNetworkString("wl_volumes")
    util.AddNetworkString("wl_wake")

    -- Buoyancy. Creates the same kind of engine object a map's
    -- water brush uses: a static physics box with the liquid's material that
    -- gets a fluid controller (waterleveler.AttachFluid). vphysics then floats
    -- props that overlap it. Created by wl_water_spawn unless it is given "nofluid".
    do
        local ENT = {}
        ENT.Type = "anim"
        ENT.Base = "base_anim"
        ENT.Spawnable = false

        function ENT:Initialize()
            self:SetModel("models/hunter/blocks/cube025x025x025.mdl")
            local h = self.WLHalf
            self:PhysicsInitBox(-h, h)
            local phys = self:GetPhysicsObject()
            if not IsValid(phys) then return print("[wl] fluid: no physics object") end
            -- The density (and so how high props float) comes from this material:
            -- "water" is what a map's water brush uses, "slime" is the slime one.
            phys:SetMaterial(self.WLLiquid or "water")
            phys:EnableMotion(false)
            -- Invisible, no shadow, and not solid to players (the engine's
            -- fluid object is a trigger for props; the entity itself isn't).
            self:SetNoDraw(true)
            self:DrawShadow(false)
            self:SetCollisionGroup(COLLISION_GROUP_DEBRIS_TRIGGER)
            local top = self:GetPos().z + h.z
            local id, err = waterleveler.AttachFluid(phys, 0, 0, 1, top, self.WLFluidOptions)
            if not id then return print("[wl] AttachFluid failed: " .. tostring(err)) end
            self.WLFluid = id
            -- Rays (crowbar, bullets, the use key) must pass through: the water
            -- is not a solid thing. The fluid controller lives on the physics
            -- object and keeps working.
            self:AddSolidFlags(FSOLID_NOT_SOLID)
            print(("[wl] fluid controller #%d attached (surface z = %.1f)"):format(id, top))
            -- State of the fluid object right after the engine made it a fluid.
            print(("[wl] fluid object: material=%s collisions=%s motion=%s solid=%d group=%d")
                :format(tostring(phys:GetMaterial()), tostring(phys:IsCollisionEnabled()),
                    tostring(phys:IsMotionEnabled()), self:GetSolid(), self:GetCollisionGroup()))
        end

        -- The controller must go before its physics object does.
        ENT.PhysgunDisabled = true
        ENT.DoNotDuplicate = true
        function ENT:OnTakeDamage() end
        function ENT:OnRemove()
            if self.WLFluid then waterleveler.DetachFluid(self.WLFluid) end
        end

        scripted_ents.Register(ENT, "wl_fluid")
    end

    -- The fluid entity is part of the water, not a prop: nothing may grab,
    -- punt, hit or tool it.
    local function isFluid(ent) return IsValid(ent) and ent:GetClass() == "wl_fluid" end
    hook.Add("PhysgunPickup", "wl_fluid_lock", function(_, ent) if isFluid(ent) then return false end end)
    hook.Add("GravGunPunt", "wl_fluid_lock", function(_, ent) if isFluid(ent) then return false end end)
    hook.Add("GravGunPickupAllowed", "wl_fluid_lock", function(_, ent) if isFluid(ent) then return false end end)
    hook.Add("CanTool", "wl_fluid_lock", function(_, tr) if isFluid(tr.Entity) then return false end end)
    hook.Add("CanProperty", "wl_fluid_lock", function(_, _, ent) if isFluid(ent) then return false end end)
    hook.Add("EntityTakeDamage", "wl_fluid_lock", function(ent) if isFluid(ent) then return true end end)

    local boxes = {} -- id -> { mins = Vector, maxs = Vector }

    -- Wakes from bullets and melee: where does a ray cross a volume's surface?
    local function surfaceCrossing(a, b)
        for _, bx in pairs(boxes) do
            local top = bx.maxs.z
            if (a.z - top) * (b.z - top) < 0 then
                local p = a + (b - a) * ((top - a.z) / (b.z - a.z))
                if p.x > bx.mins.x and p.x < bx.maxs.x and p.y > bx.mins.y and p.y < bx.maxs.y then
                    return p
                end
            end
        end
    end

    -- Where one ray (stopped by the first solid hit) crosses a volume's surface.
    local function wakePoint(from, to, attacker)
        -- Something solid in the way takes the hit instead of the water.
        local tr = util.TraceLine({ start = from, endpos = to, filter = attacker, mask = MASK_SHOT })
        if tr.Hit then to = tr.HitPos end
        return surfaceCrossing(from, to)
    end

    local function sendWakes(points)
        if #points == 0 then return end
        net.Start("wl_wake")
        net.WriteUInt(#points, 8)
        for _, p in ipairs(points) do net.WriteVector(p) end
        net.Broadcast()
    end

    local function sendWake(from, to, attacker)
        if next(boxes) == nil then return end
        local p = wakePoint(from, to, attacker)
        if p then sendWakes({ p }) end
    end

    -- Every bullet makes its own wake: a shotgun blast is several rays, each
    -- spread inside the weapon's cone like the engine does (data.Spread is the
    -- cone's half-extent on each axis).
    hook.Add("EntityFireBullets", "wl_wake_bullets", function(ent, data)
        if next(boxes) == nil then return end
        local dist = math.min(data.Distance or 56756, 56756)
        local dir = data.Dir:GetNormalized()
        local right = dir:Cross(Vector(0, 0, 1))
        if right:LengthSqr() < 1e-6 then right = Vector(0, 1, 0) end
        right:Normalize()
        local up = right:Cross(dir)
        local spread = data.Spread or vector_origin
        local points = {}
        for i = 1, math.Clamp(data.Num or 1, 1, 64) do
            local d = dir
            if i > 1 or (data.Num or 1) > 1 then
                d = (dir + right * spread.x * math.Rand(-1, 1) + up * spread.y * math.Rand(-1, 1)):GetNormalized()
            end
            local p = wakePoint(data.Src, data.Src + d * dist, ent)
            if p then points[#points + 1] = p end
        end
        sendWakes(points)
    end)

    -- Melee: a swing is a new next-primary-fire time on a melee weapon.
    local lastSwing = setmetatable({}, { __mode = "k" })
    hook.Add("Think", "wl_wake_melee", function()
        if next(boxes) == nil then return end
        for _, ply in ipairs(player.GetAll()) do
            local wep = ply:GetActiveWeapon()
            local cls = IsValid(wep) and wep:GetClass() or ""
            if cls:find("crowbar") or cls:find("stunstick") or cls:find("melee") then
                local nextFire = wep:GetNextPrimaryFire()
                if lastSwing[wep] ~= nextFire then
                    lastSwing[wep] = nextFire
                    if ply:KeyDown(IN_ATTACK) or ply:KeyDown(IN_ATTACK2) then
                        sendWake(ply:EyePos(), ply:EyePos() + ply:GetAimVector() * 80, ply)
                    end
                end
            end
        end
    end)

    local fluids = {}


    local function sync(ply)
        net.Start("wl_volumes")
        net.WriteUInt(table.Count(boxes), 16)
        for _, b in pairs(boxes) do
            net.WriteVector(b.mins)
            net.WriteVector(b.maxs)
            net.WriteString(b.surface or "") -- "" = the default material
            net.WriteString(b.body or "")
        end
        if IsValid(ply) then net.Send(ply) else net.Broadcast() end
    end

    --- Create a volume (engine water + optional buoyancy + undo entry).
    --- opts: liquid ("water" default or "slime"; it also picks the fluid's physics material),
    --- fluid (default true), surface / body (material names, "" = the default), fluidOptions
    --- (the options table of waterleveler.AttachFluid: damping, torque, viscosity, aerodynamics,
    --- currentx/y/z).
    --- Returns the volume id, or nil, err.
    function WLWater.Spawn(ply, mins, maxs, opts)
        opts = opts or {}
        if opts.liquid ~= "slime" then opts.liquid = "water" end
        local id, err = waterleveler.AddVolume(mins.x, mins.y, mins.z, maxs.x, maxs.y, maxs.z, opts.liquid)
        if not id then return nil, err end

        boxes[id] = { mins = mins, maxs = maxs, surface = opts.surface, body = opts.body }
        local fluid
        if opts.fluid ~= false then
            local e = ents.Create("wl_fluid")
            if IsValid(e) then
                e.WLHalf = (maxs - mins) / 2
                local fo = table.Copy(opts.fluidOptions or {}) -- empty = a map water brush's values
                fo.contents = opts.liquid
                e.WLFluidOptions = fo
                e.WLLiquid = opts.liquid
                e:SetPos((mins + maxs) / 2)
                e:Spawn()
                fluids[#fluids + 1] = e
                fluid = e
            end
        end
        sync()

        -- Undo (Z / the undo menu) removes this volume and its fluid again.
        if IsValid(ply) then
            local size = maxs - mins
            undo.Create("Water volume")
            undo.SetPlayer(ply)
            undo.AddFunction(function(_, volume_id, fluid_ent)
                if IsValid(fluid_ent) then fluid_ent:Remove() end -- detaches the controller
                if boxes[volume_id] then
                    waterleveler.RemoveVolume(volume_id)
                    boxes[volume_id] = nil
                    sync()
                end
            end, id, fluid)
            undo.SetCustomUndoText("Undone water volume")
            undo.Finish(("Water volume (%dx%dx%d)"):format(size.x, size.y, size.z))
        end
        return id
    end

    function WLWater.Allowed(ply)
        return not IsValid(ply) or ply:IsSuperAdmin() or game.SinglePlayer() or ply:IsListenServerHost()
    end

    local function resolve(ply)
        if not IsValid(ply) then ply = player.GetAll()[1] end
        return ply
    end

    local allowed = WLWater.Allowed

    concommand.Add("wl_water_spawn", function(caller, _, args)
        if not allowed(caller) then return end
        local ply = resolve(caller)
        if not IsValid(ply) then return print("[wl] no player to aim from") end

        local size = math.Clamp(tonumber(args[1]) or SIZE, MIN_SIZE, MAX_SIZE)
        local tr = util.TraceLine({
            start = ply:EyePos(),
            endpos = ply:EyePos() + ply:GetAimVector() * 16384,
            filter = ply,
        })
        if not tr.Hit then return print("[wl] crosshair does not hit anything") end

        -- Rest the box on the surface under the crosshair.
        local mins, maxs, center = WLWater.Placement(tr, Vector(size, size, size))
        local id, err = WLWater.Spawn(ply, mins, maxs, { fluid = args[2] ~= "nofluid" })
        if not id then return print("[wl] AddVolume failed: " .. tostring(err)) end
        print(("[wl] water volume #%d spawned, %dx%dx%d at %s"):format(id, size, size, size, tostring(center)))
    end)

    concommand.Add("wl_water_clear", function(caller)
        if not allowed(caller) then return end
        for _, e in ipairs(fluids) do if IsValid(e) then e:Remove() end end -- detaches the controller
        fluids = {}
        waterleveler.ClearVolumes()
        boxes = {}
        sync()
        print("[wl] all water volumes removed")
    end)

    concommand.Add("wl_water_status", function(caller)
        print("[wl] " .. waterleveler.HookStatus())
        print(("[wl] contents hook: %s | volumes: %d | point queries answered with water: %d | traces stopped at the surface: %d | bubble trails clipped: %d | explosives forced into water: %d")
            :format(tostring(waterleveler.IsContentsHooked()), waterleveler.VolumeCount(),
                waterleveler.ContentsHits(), waterleveler.TraceHits(), waterleveler.BubbleClips(), waterleveler.ExplosionFixes()))
        print(("[wl] explosion events sent as the water explosion: %d"):format(waterleveler.ExplosionsRedirected()))
        print(("[wl] fluid controllers: %d"):format(waterleveler.FluidCount()))
        local ply = resolve(caller)
        if IsValid(ply) then
            local p = ply:GetPos()
            print(("[wl] %s: WaterLevel() = %d, feet inside a volume: %s")
                :format(ply:Nick(), ply:WaterLevel(), tostring(waterleveler.PointInVolume(p.x, p.y, p.z))))
        end
    end)

    hook.Add("PlayerInitialSpawn", "wl_volumes_sync", function(ply)
        timer.Simple(2, function() if IsValid(ply) then sync(ply) end end)
    end)
end

if CLIENT then
    -- Client half of the networked fluid entity (invisible; the volume is drawn below).
    scripted_ents.Register({ Type = "anim", Base = "base_anim", Spawnable = false, Draw = function() end }, "wl_fluid")

    local boxes = {}
    local syncVolumeEntities -- defined with the drawing code below

    net.Receive("wl_volumes", function()
        boxes = {}
        for i = 1, net.ReadUInt(16) do
            boxes[i] = { mins = net.ReadVector(), maxs = net.ReadVector(), surface = net.ReadString(), body = net.ReadString() }
        end
        syncVolumeEntities()
    end)

    -- The Water Volume tool picks the materials of the top face and of the
    -- bottom and sides. Any material path works, for example
    -- nature/water_movingplane or models/shadertest/predator. A name that does
    -- not resolve falls back to a translucent blue so the volume never disappears.

    local fallback = Material("color") -- plain white, tinted by the color below
    local cache = {}
    local unlitCount = 0

    -- World textures (LightmappedGeneric etc.) need lightmap data a dynamic
    -- mesh does not have, which renders as black faces and blocky garbage.
    -- Only shaders that work on plain meshes are used as they are (VertexLitGeneric
    -- too, e.g. models/shadertest/*: its envmap, bump map and alpha masks are
    -- authored for that shader, and re-wrapping it showed them as raw texture
    -- alpha: pink with holes); anything
    -- else is re-wrapped as UnlitGeneric using the same base texture.
    local passthrough = { refract = true, unlitgeneric = true, spritecard = true, vertexlitgeneric = true }

    local function mat(name)
        local entry = cache[name]
        if entry == nil then
            -- "smooth mips noclamp" only applies to image files (png/jpg). VMT
            -- materials get their filtering and mipmaps from their VTF, so
            -- they are loaded as-is and the draw call forces filtering below.
            local lower = name:lower()
            local isImage = lower:match("%.png$") or lower:match("%.jpe?g$")
            local m = isImage and Material(name, "smooth mips noclamp") or Material(name)
            if m:IsError() then
                entry = { mat = fallback, color = Color(40, 120, 220, 90), vcol = false }
            else
                -- GetShader() reports the DX level too ("Refract_DX90", "Water_DX90").
                local shader = (m:GetShader() or ""):lower():gsub("_dx%d+$", "")
                -- Texture parameters are read as strings: GetTexture() warns
                -- ("not a texture value") for vars a shader does not declare.
                local nm, dudv = m:GetString("$normalmap"), m:GetString("$dudvmap")
                if shader == "water" and nm and nm ~= "" then
                    -- The Water shader draws its reflection and refraction only for the
                    -- world's own water surfaces; on a mesh the animated normal map
                    -- (dev/water_normal, advanced by $bumpframe) never shows. A Refract
                    -- material with the same normal map and the same animation does.
                    unlitCount = unlitCount + 1
                    entry = { mat = CreateMaterial("wl_water_" .. unlitCount, "Refract", {
                        ["$normalmap"] = nm,
                        ["$refractamount"] = 0.1,
                        ["$bluramount"] = 0,
                        ["$scale"] = "[1 1]",
                        ["$refracttint"] = "{190 215 255}",
                        Proxies = { AnimatedTexture = {
                            animatedtexturevar = "$normalmap",
                            animatedtextureframenumvar = "$bumpframe",
                            animatedtextureframerate = 30 } },
                    }), color = color_white, vcol = false, cull = true, refract = true }
                elseif shader == "refract" and m:GetInt("$model") == 1 and nm and nm ~= "" then
                    -- Refract materials flagged "$model 1" (models/shadertest/shader3, ...)
                    -- are built for model vertex data and an env_cubemap; on a plain
                    -- mesh they come out pink with holes. Same refraction, minus those.
                    unlitCount = unlitCount + 1
                    local params = {
                        ["$normalmap"] = nm,
                        ["$refractamount"] = m:GetFloat("$refractamount") or 0.25,
                        ["$bluramount"] = m:GetInt("$bluramount") or 1,
                        ["$scale"] = "[1 1]",
                        Proxies = { TextureScroll = {
                            texturescrollvar = "$bumptransform", texturescrollrate = 0.3, texturescrollangle = -90 } },
                    }
                    if dudv and dudv ~= "" then params["$dudvmap"] = dudv end
                    local tint = m:GetVector("$refracttint")
                    if tint then
                        params["$refracttint"] = ("{%d %d %d}"):format(tint.x * 255, tint.y * 255, tint.z * 255)
                    end
                    entry = { mat = CreateMaterial("wl_refract_" .. unlitCount, "Refract", params),
                        color = color_white, vcol = false, cull = true, refract = true }
                elseif passthrough[shader] then
                    -- UnlitGeneric (png/jpg) takes its alpha from the vertex colour.
                    entry = { mat = m, color = color_white, vcol = shader == "unlitgeneric", refract = shader == "refract" }
                else
                    local tex = m:GetTexture("$basetexture")
                    if tex and not tex:IsError() then
                        unlitCount = unlitCount + 1
                        local um = CreateMaterial("wl_unlit_" .. unlitCount, "UnlitGeneric", {
                            ["$basetexture"] = tex:GetName(),
                            ["$vertexcolor"] = 1,
                            ["$vertexalpha"] = 1,
                            ["$translucent"] = 1,
                            ["$nocull"] = 1,
                        })
                        entry = { mat = um, color = Color(255, 255, 255, 200), vcol = true }
                    else
                        entry = { mat = fallback, color = Color(40, 120, 220, 90), vcol = false }
                    end
                end
            end
            cache[name] = entry
        end
        return entry
    end

    local TILE = 128 -- world units per texture repeat

    -- One tiled quad, a -> b -> c -> d. Every vertex carries a colour, a normal
    -- and tangents: leaving them unset makes lit or shader-driven materials
    -- (including water) flicker or render black.
    local function quad(a, b, c, d, uLen, vLen, cr, cg, cb, ca, outward)
        local u, v = uLen / TILE, vLen / TILE
        local s = (b - a):GetNormalized()
        local t = (d - a):GetNormalized()
        local n = s:Cross(t)
        local verts = { { a, 0, 0 }, { b, u, 0 }, { c, u, v }, { d, 0, v } }
        if outward then
            -- Culled materials: the engine's front face is the clockwise one,
            -- so wind the quad clockwise as seen from outside the box.
            if n:Dot(outward) > 0 then
                verts = { verts[4], verts[3], verts[2], verts[1] }
            end
            n = outward -- lighting normal points out of the box
        end
        mesh.Begin(MATERIAL_QUADS, 1)
        for _, vert in ipairs(verts) do
            mesh.Position(vert[1])
            mesh.Color(cr, cg, cb, ca)
            mesh.Normal(n)
            mesh.TangentS(s)
            mesh.TangentT(t)
            mesh.TexCoord(0, vert[2], vert[3])
            mesh.AdvanceVertex()
        end
        mesh.End()
    end

    local function face(entry, a, b, c, d, uLen, vLen, outward)
        render.SetMaterial(entry.mat)
        render.CullMode(entry.cull and MATERIAL_CULLMODE_CCW or MATERIAL_CULLMODE_NONE)
        if not entry.cull then outward = nil end
        local col = entry.color
        if entry.vcol then
            -- Colour and alpha travel in the vertices.
            render.SetColorModulation(1, 1, 1)
            render.SetBlend(1)
            quad(a, b, c, d, uLen, vLen, col.r, col.g, col.b, col.a, outward)
        else
            render.SetColorModulation(col.r / 255, col.g / 255, col.b / 255)
            render.SetBlend(col.a / 255)
            quad(a, b, c, d, uLen, vLen, 255, 255, 255, 255, outward)
        end
    end

    -- Each volume is drawn by a client-side entity in the engine's translucent
    -- sort, so it is ordered against splashes, sprites and shards by distance
    -- like any other translucent thing (origin = the box centre).
    local function drawVolume(bx)
        local top = mat(bx.surface ~= "" and bx.surface or DEFAULT_MATERIAL)
        local body = mat(bx.body ~= "" and bx.body or DEFAULT_MATERIAL)
        render.ResetModelLighting(1, 1, 1) -- VertexLitGeneric materials: even, full light
        -- Refract materials sample a copy of the frame, which is only updated when
        -- asked to: a stale copy is what made their faces flicker.
        if top.refract or body.refract then render.UpdateRefractTexture() end
        -- Smooth filtering with mipmaps (anisotropic = trilinear + aniso).
        render.PushFilterMag(TEXFILTER.ANISOTROPIC)
        render.PushFilterMin(TEXFILTER.ANISOTROPIC)
        do
            -- Inset the bottom and sides so they are not coplanar with the
            -- floor or walls the volume rests against (z-fighting).
            local e = 0.5
            local mn = Vector(bx.mins.x + e, bx.mins.y + e, bx.mins.z + e)
            local mx = Vector(bx.maxs.x - e, bx.maxs.y - e, bx.maxs.z)
            local dx, dy, dz = mx.x - mn.x, mx.y - mn.y, mx.z - mn.z
            local p000, p100 = Vector(mn.x, mn.y, mn.z), Vector(mx.x, mn.y, mn.z)
            local p110, p010 = Vector(mx.x, mx.y, mn.z), Vector(mn.x, mx.y, mn.z)
            local p001, p101 = Vector(mn.x, mn.y, mx.z), Vector(mx.x, mn.y, mx.z)
            local p111, p011 = Vector(mx.x, mx.y, mx.z), Vector(mn.x, mx.y, mx.z)
            -- Sides and bottom first, surface last, so the surface blends over them.
            face(body, p000, p100, p110, p010, dx, dy, Vector(0, 0, -1)) -- bottom
            face(body, p000, p100, p101, p001, dx, dz, Vector(0, -1, 0)) -- -Y side
            face(body, p010, p110, p111, p011, dx, dz, Vector(0, 1, 0)) -- +Y side
            face(body, p000, p010, p011, p001, dy, dz, Vector(-1, 0, 0)) -- -X side
            face(body, p100, p110, p111, p101, dy, dz, Vector(1, 0, 0)) -- +X side
            face(top, p001, p101, p111, p011, dx, dy, Vector(0, 0, 1))  -- surface
        end
        render.PopFilterMin()
        render.PopFilterMag()
        render.CullMode(MATERIAL_CULLMODE_CCW)
        render.SetColorModulation(1, 1, 1)
        render.SetBlend(1)
    end

    -- Also used by the Water Volume tool for its preview.
    WLWater.DrawVolume = drawVolume

    scripted_ents.Register({
        Type = "anim",
        Base = "base_anim",
        Spawnable = false,
        RenderGroup = RENDERGROUP_TRANSLUCENT,
        Initialize = function(self)
            self:SetRenderMode(RENDERMODE_TRANSALPHA)
        end,
        Draw = function(self)
            if self.WLBox then drawVolume(self.WLBox) end
        end,
    }, "wl_volume_draw")

    local volumeEnts = {}

    syncVolumeEntities = function()
        for _, e in ipairs(volumeEnts) do
            if IsValid(e) then e:Remove() end
        end
        volumeEnts = {}
        for _, bx in ipairs(boxes) do
            local e = ents.CreateClientside("wl_volume_draw")
            if IsValid(e) then
                local centre = (bx.mins + bx.maxs) * 0.5
                e:SetModel("models/error.mdl")
                e:SetPos(centre)
                e:Spawn()
                e.WLBox = bx
                e:SetRenderBounds(bx.mins - centre, bx.maxs - centre)
                volumeEnts[#volumeEnts + 1] = e
            end
        end
    end

    -- Map cleanups remove client-side entities too.
    hook.Add("PostCleanupMap", "wl_volumes_entities", function() syncVolumeEntities() end)

    -- Wake ripples. The engine's own ripple (effects/splashwake1) is spawned
    -- by client code that does its own water trace, which a volume cannot
    -- feed, so the same material is drawn here as flat, growing, fading quads
    -- on the surface wherever something moves through it.
    local rippleMat = Material("effects/splashwake1")
    local ripples = {}
    local straddling = {} -- entindex -> true while the entity crosses the surface
    local RIPPLE_LIFE = 1.5

    -- Engine values (client FX_WaterRipple, read from client.dll): the effect's
    -- scale (10-12 for the "waterripple" a player makes) is multiplied by 0.125, the
    -- quad grows from scale*16 to scale*128 units over 1.5 s with a size bias of 0.7,
    -- fades from an alpha taken from the light at the spot (luma*4, clamped to
    -- 0.25-1) to 0 with an alpha bias of 0.25, and is tinted 0.75 + 0.25*light.
    -- The engine's physics splash ignores anything slower than 25 units/s (it measures
    -- the full velocity, not just z).
    local MIN_SPEED = 25

    local function bias(x, b) return x ^ (math.log(b) / math.log(0.5)) end

    local function addRipple(pos)
        local light = render.GetLightColor(pos + Vector(0, 0, 4))
        local luma = light.x * 0.3 + light.y * 0.59 + light.z * 0.11
        local m = math.max(light.x, light.y, light.z)
        if m > 0 then light = light / m end
        ripples[#ripples + 1] = {
            pos = pos, born = CurTime(), rot = math.random(0, 359),
            scale = math.Rand(10, 12) * 0.125,
            alpha = math.Clamp(luma * 4, 0.25, 1),
            tint = Color(255 * (0.75 + 0.25 * light.x), 255 * (0.75 + 0.25 * light.y), 255 * (0.75 + 0.25 * light.z)),
        }
        if #ripples > 200 then table.remove(ripples, 1) end
    end

    -- Where the segment a -> b crosses the top face of a volume, or nil.
    local function surfaceCrossing(a, b)
        for _, bx in ipairs(boxes) do
            local top = bx.maxs.z
            if (a.z - top) * (b.z - top) < 0 then
                local t = (top - a.z) / (b.z - a.z)
                local p = a + (b - a) * t
                if p.x > bx.mins.x and p.x < bx.maxs.x and p.y > bx.mins.y and p.y < bx.maxs.y then
                    return p
                end
            end
        end
    end

    -- Wakes from bullet impacts and melee swings are found on the server
    -- (it sees every shot and swing) and sent here.
    net.Receive("wl_wake", function()
        for _ = 1, net.ReadUInt(8) do addRipple(net.ReadVector()) end
    end)

    hook.Add("Think", "wl_volumes_ripples", function()
        local now = CurTime()
        -- Expire first: with no volumes left nothing else would ever remove them.
        for i = #ripples, 1, -1 do
            if now - ripples[i].born > RIPPLE_LIFE then table.remove(ripples, i) end
        end
        if #boxes == 0 then
            ripples = {}
            straddling = {}
            return
        end
        local seen = {}
        for _, bx in ipairs(boxes) do
            local top = bx.maxs.z
            local lo = Vector(bx.mins.x, bx.mins.y, top - 128)
            local hi = Vector(bx.maxs.x, bx.maxs.y, top + 128)
            for _, ent in ipairs(ents.FindInBox(lo, hi)) do
                local cls = ent:GetClass()
                if cls ~= "wl_fluid" and cls ~= "worldspawn" and not ent:IsWeapon() and ent:GetNoDraw() == false
                    -- Noclipping players and spectators are not in the water.
                    and not (ent:IsPlayer() and (ent:GetMoveType() == MOVETYPE_NOCLIP or ent:GetObserverMode() ~= OBS_MODE_NONE))
                    and (ent:IsPlayer() or ent:IsNPC() or ent:IsRagdoll() or cls:find("^prop_")) then
                    local idx = ent:EntIndex()
                    local pos = ent:GetPos()
                    local omin, omax = ent:OBBMins(), ent:OBBMaxs()
                    local inside = pos.x > bx.mins.x and pos.x < bx.maxs.x and pos.y > bx.mins.y and pos.y < bx.maxs.y
                    local cross = inside and pos.z + omin.z <= top and pos.z + omax.z >= top
                    local fast = ent:GetVelocity():Length() >= MIN_SPEED
                    local at = Vector(pos.x, pos.y, top)
                    if cross then
                        seen[idx] = true
                        -- One wake when it breaks the surface fast enough, none otherwise.
                        if not straddling[idx] and fast then addRipple(at) end
                    elseif straddling[idx] and inside and fast then
                        addRipple(at) -- left through the surface
                    end
                end
            end
        end
        straddling = seen
    end)

    hook.Add("PostDrawTranslucentRenderables", "wl_volumes_ripples_draw", function(_, skybox)
        if skybox or #ripples == 0 then return end
        render.SetMaterial(rippleMat)
        render.PushFilterMag(TEXFILTER.ANISOTROPIC)
        render.PushFilterMin(TEXFILTER.ANISOTROPIC)
        local now = CurTime()
        for _, r in ipairs(ripples) do
            local t = (now - r.born) / RIPPLE_LIFE
            local size = Lerp(bias(t, 0.7), 16 * r.scale, 128 * r.scale)
            local a = 255 * Lerp(bias(t, 0.25), r.alpha, 0)
            local c = r.tint
            render.DrawQuadEasy(r.pos + Vector(0, 0, 0.5), Vector(0, 0, 1), size, size, Color(c.r, c.g, c.b, a), r.rot)
        end
        render.PopFilterMin()
        render.PopFilterMag()
    end)

    -- Fully submerged (water level 3): the engine's water screen overlay.
    hook.Add("RenderScreenspaceEffects", "wl_volumes_overlay", function()
        local ply = LocalPlayer()
        if not IsValid(ply) or not ply:Alive() then return end
        if ply:WaterLevel() >= 3 then
            DrawMaterialOverlay("effects/water_warp01", 0.1)
        end
    end)
end
