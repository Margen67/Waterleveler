-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- waterleveler_server.lua: the server half of the Water Volume script (volume entities, undo and
-- cleanup, networking, the debug commands). Shared parts: autorun/waterleveler_volumes.lua.
require("waterleveler");
util.AddNetworkString("wl_volumes");
-- Buoyancy. Creates the same kind of engine object a map's
-- water brush uses: a static physics box with the liquid's material that
-- gets a fluid controller (waterleveler.AttachFluid). vphysics then floats
-- props that overlap it. Every volume has one as its handle (undo and cleanup remove it, and the volume with it); the buoyancy is only attached unless the tool's buoyancy option is off.
do
    local fluidEnt = {};
    fluidEnt.Type = "anim";
    fluidEnt.Base = "base_anim";
    fluidEnt.Spawnable = false;
    function fluidEnt:Initialize()
        self:SetModel("models/hunter/blocks/cube025x025x025.mdl");
        self:SetNoDraw(true);
        self:DrawShadow(false);
        -- Without buoyancy this entity is only the volume's handle (undo, cleanup).
        if not self.WLFluidOptions then return; end
        local h = self.WLHalf;
        self:PhysicsInitBox(-h, h);
        local phys = self:GetPhysicsObject();
        if not IsValid(phys) then return print("[wl] fluid: no physics object"); end
        -- The density (and so how high props float) comes from this material:
        -- "water" is what a map's water brush uses, "slime" is the slime one.
        phys:SetMaterial(self.WLLiquid or "water");
        phys:EnableMotion(false);
        -- Invisible, no shadow, and not solid to players (the engine's
        -- fluid object is a trigger for props; the entity itself isn't).
        self:SetNoDraw(true);
        self:DrawShadow(false);
        self:SetCollisionGroup(COLLISION_GROUP_DEBRIS_TRIGGER);
        local top = self:GetPos().z + h.z;
        local id, err = waterleveler.AttachFluid(phys, 0, 0, 1, top, self.WLFluidOptions);
        if not id then return print("[wl] AttachFluid failed: " .. tostring(err)); end
        self.WLFluid = id;
        -- Rays (crowbar, bullets, the use key) must pass through: the water
        -- is not a solid thing. The fluid controller lives on the physics
        -- object and keeps working.
        self:AddSolidFlags(FSOLID_NOT_SOLID);
        if GetConVar("developer"):GetInt() > 0 then
            print(("[wl] fluid controller #%d attached (surface z = %.1f)"):format(id, top));
            -- State of the fluid object right after the engine made it a fluid.
            print(("[wl] fluid object: material=%s collisions=%s motion=%s solid=%d group=%d"):format(tostring(phys:GetMaterial()), tostring(phys:IsCollisionEnabled()), tostring(phys:IsMotionEnabled()), self:GetSolid(), self:GetCollisionGroup()));
        end
    end

    -- The controller must go before its physics object does.
    fluidEnt.PhysgunDisabled = true;
    fluidEnt.DoNotDuplicate = true;
    function fluidEnt:OnTakeDamage()
    end

    function fluidEnt:OnRemove()
        if self.WLFluid then waterleveler.DetachFluid(self.WLFluid); end
        -- The entity is the volume's handle: removing it (undo, cleanup) removes the volume.
        if self.WLVolumeId then WLWater.RemoveBox(self.WLVolumeId); end
    end

    scripted_ents.Register(fluidEnt, "wl_fluid");
end

-- The fluid entity is part of the water, not a prop: nothing may grab,
-- punt, hit or tool it.
local function isFluid(ent)
    return IsValid(ent) and ent:GetClass() == "wl_fluid";
end

hook.Add("PhysgunPickup", "wl_fluid_lock", function(_, ent) if isFluid(ent) then return false; end end);
hook.Add("GravGunPunt", "wl_fluid_lock", function(_, ent) if isFluid(ent) then return false; end end);
hook.Add("GravGunPickupAllowed", "wl_fluid_lock", function(_, ent) if isFluid(ent) then return false; end end);
hook.Add("CanTool", "wl_fluid_lock", function(_, tr) if isFluid(tr.Entity) then return false; end end);
hook.Add("CanProperty", "wl_fluid_lock", function(_, _, ent) if isFluid(ent) then return false; end end);
hook.Add("EntityTakeDamage", "wl_fluid_lock", function(ent) if isFluid(ent) then return true; end end);
local boxes = {}; -- id -> { mins = Vector, maxs = Vector }
local function sync(ply)
    net.Start("wl_volumes");
    net.WriteUInt(table.Count(boxes), 16);
    for _, b in pairs(boxes) do
        net.WriteVector(b.mins);
        net.WriteVector(b.maxs);
        net.WriteString(b.material or ""); -- "" = the default material
        net.WriteString(b.liquid or "water");
        net.WriteBool(b.beneath ~= false);
    end

    if IsValid(ply) then
        net.Send(ply);
    else
        net.Broadcast();
    end
end

--- Removes a volume (by id) from the module and the clients. Used when its entity goes.
function WLWater.RemoveBox(id)
    if not boxes[id] then return; end
    waterleveler.RemoveVolume(id);
    boxes[id] = nil;
    sync();
end

--- Create a volume (engine water + optional buoyancy + undo entry).
--- opts: liquid ("water" default or "slime"; it also picks the fluid's physics material),
--- fluid (default true), material (name, "" = the default), fluidOptions
--- (the options table of waterleveler.AttachFluid: damping, torque, viscosity, aerodynamics,
--- currentx/y/z).
--- Returns the volume id, or nil, err.
function WLWater.Spawn(ply, mins, maxs, opts)
    opts = opts or {};
    opts.liquid = tostring(opts.liquid or ""):lower() == "slime" and "slime" or "water";
    local id, err = waterleveler.AddVolume(mins.x, mins.y, mins.z, maxs.x, maxs.y, maxs.z, opts.liquid);
    if not id then return nil, err; end
    boxes[id] = {
        mins = mins,
        maxs = maxs,
        material = opts.material,
        beneath = opts.beneath ~= false,
        liquid = opts.liquid
    };

    -- Every volume has a wl_fluid entity: it is what undo and cleanup remove (the volume
    -- goes with it), and it carries the buoyancy unless that is switched off.
    local e = ents.Create("wl_fluid");
    if not IsValid(e) then
        waterleveler.RemoveVolume(id);
        boxes[id] = nil;
        return nil, "could not create the volume's entity";
    end

    e.WLHalf = (maxs - mins) / 2;
    e.WLLiquid = opts.liquid;
    e.WLVolumeId = id;
    if opts.fluid ~= false then
        local fo = table.Copy(opts.fluidOptions or {}); -- empty = a map water brush's values
        fo.contents = opts.liquid;
        e.WLFluidOptions = fo;
    end

    e:SetPos((mins + maxs) / 2);
    e:Spawn();
    sync();
    if IsValid(ply) then
        local size = maxs - mins;
        undo.Create("Water volume");
        undo.SetPlayer(ply);
        undo.AddEntity(e);
        undo.SetCustomUndoText("Undone water volume");
        undo.Finish(("Water volume (%dx%dx%d)"):format(size.x, size.y, size.z));
        ply:AddCleanup("waterleveler", e);
    end
    return id;
end

function WLWater.Allowed(ply)
    return not IsValid(ply) or ply:IsSuperAdmin() or game.SinglePlayer() or ply:IsListenServerHost();
end

local function resolve(ply)
    if not IsValid(ply) then ply = player.GetAll()[1]; end
    return ply;
end

local allowed = WLWater.Allowed;
concommand.Add("waterleveler_status", function(caller)
    print("[wl] " .. waterleveler.HookStatus());
    print(("[wl] contents hook: %s | volumes: %d | point queries answered with water: %d | traces stopped at the surface: %d | bubble trails clipped: %d | explosives forced into water: %d"):format(tostring(waterleveler.IsContentsHooked()), waterleveler.VolumeCount(), waterleveler.ContentsHits(), waterleveler.TraceHits(), waterleveler.BubbleClips(), waterleveler.ExplosionFixes()));
    print(("[wl] explosion events sent as the water explosion: %d"):format(waterleveler.ExplosionsRedirected()));
    print(("[wl] fluid controllers: %d"):format(waterleveler.FluidCount()));
    local ply = resolve(caller);
    if IsValid(ply) then
        local p = ply:GetPos();
        print(("[wl] %s: WaterLevel() = %d, feet inside a volume: %s"):format(ply:Nick(), ply:WaterLevel(), tostring(waterleveler.PointInVolume(p.x, p.y, p.z))));
    end
end);

concommand.Add("waterleveler_probe", function(caller)
    local ply = resolve(caller);
    if not IsValid(ply) then return; end
    local mins, maxs = ply:GetHull();
    local origin = ply:GetPos();
    local points = {{"feet", origin + Vector(0, 0, mins.z + 1)}, {"waist", origin + (mins + maxs) * 0.5}, {"eyes", ply:EyePos()}};
    -- The volume the player is in, else the one whose box is nearest to the eyes.
    local best, bestDistance;
    for _, b in pairs(boxes) do
        local eyes = points[3][2];
        local dx = math.max(b.mins.x - eyes.x, 0, eyes.x - b.maxs.x);
        local dy = math.max(b.mins.y - eyes.y, 0, eyes.y - b.maxs.y);
        local dz = math.max(b.mins.z - eyes.z, 0, eyes.z - b.maxs.z);
        local distance = dx * dx + dy * dy + dz * dz;
        if not bestDistance or distance < bestDistance then best, bestDistance = b, distance; end
    end

    print(("[wl] %s: WaterLevel() = %d, hull %s to %s, ducking: %s"):format(ply:Nick(), ply:WaterLevel(), tostring(mins), tostring(maxs), tostring(ply:Crouching())));
    if not best then
        print("[wl] no volumes");
        return;
    end

    print(("[wl] nearest volume: mins %s, maxs %s (surface z = %.2f)"):format(tostring(best.mins), tostring(best.maxs), best.maxs.z));
    for _, point in ipairs(points) do
        local p = point[2];
        print(("[wl]   %-5s z = %.2f, %.2f below the surface, inside a volume (module): %s, inside this box (script): %s"):format(point[1], p.z, best.maxs.z - p.z, tostring(waterleveler.PointInVolume(p.x, p.y, p.z)), tostring(p:WithinAABox(best.mins, best.maxs))));
    end
end);

concommand.Add("waterleveler_explode", function(caller)
    if not allowed(caller) then return; end
    local ply = resolve(caller);
    if not IsValid(ply) then return; end
    local tr = util.TraceLine({
        start = ply:EyePos(),
        endpos = ply:EyePos() + ply:GetAimVector() * 32768,
        filter = ply
    });

    local pos = tr.HitPos + tr.HitNormal * 8;
    local e = ents.Create("env_explosion");
    if not IsValid(e) then return; end
    e:SetPos(pos);
    e:SetKeyValue("iMagnitude", "100");
    e:SetOwner(ply);
    e:Spawn();
    e:Fire("Explode", "", 0);
    e:Fire("Kill", "", 1);
    print(("[wl] env_explosion at %s, inside a volume: %s, redirected so far: %d"):format(tostring(pos), tostring(waterleveler.PointInVolume(pos.x, pos.y, pos.z)), waterleveler.ExplosionsRedirected()));
end);

-- "Clean up everything" (game.CleanUpMap) removes the fluid entities; the volumes
-- themselves live in the module and the script, so they go too.
hook.Add("PostCleanupMap", "wl_volumes_cleanup", function()
    waterleveler.ClearVolumes();
    boxes = {};
    sync();
end);

hook.Add("PlayerInitialSpawn", "wl_volumes_sync", function(ply) timer.Simple(2, function() if IsValid(ply) then sync(ply); end end); end);
