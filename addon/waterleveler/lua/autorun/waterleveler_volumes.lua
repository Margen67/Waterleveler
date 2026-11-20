-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- waterleveler_volumes.lua
-- Put this in garrysmod/lua/autorun/  (NOT autorun/server - it is shared, the
-- client half draws the placeholder box).
--
-- Volumes are created with the Water Volume tool (Tools > Construction) or
-- the waterleveler library. Debugging commands:
--   waterleveler_status         (server) hook status, counters, your water level
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
local defaultMaterial = "nature/water_movingplane"; -- when a volume names none
WLWater = WLWater or {};
--- Where a volume of `size` (Vector) goes for the trace `tr`: it rests against the surface the
--- trace hit (floor, wall or ceiling), then moves `offset` units up (down when negative).
--- Returns mins, maxs. Shared by the tool's preview (client) and the spawn (server).
function WLWater.Placement(tr, size, offset)
    local half = size * 0.5;
    local n = tr.HitNormal;
    local center = tr.HitPos + Vector(n.x * half.x, n.y * half.y, n.z * half.z);
    center.z = center.z + (offset or 0);
    return center - half, center + half, center;
end

if SERVER then
    AddCSLuaFile();
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
end

if CLIENT then
    -- Client half of the networked fluid entity (invisible; the volume is drawn below).
    scripted_ents.Register({
        Type = "anim",
        Base = "base_anim",
        Spawnable = false,
        Draw = function() end
    }, "wl_fluid");

    local boxes = {};
    local syncVolumeEntities; -- defined with the drawing code below
    net.Receive("wl_volumes", function()
        boxes = {};
        for i = 1, net.ReadUInt(16) do
            boxes[i] = {
                mins = net.ReadVector(),
                maxs = net.ReadVector(),
                material = net.ReadString(),
                liquid = net.ReadString(),
                beneath = net.ReadBool()
            };
        end

        -- The client module (gmcl_waterlevelercore) makes the client's own water queries see the
        -- volumes: ripples and splashes, the underwater sound filter, particles, prediction.
        if waterlevelercore then
            waterlevelercore.ClearVolumes();
            for _, b in ipairs(boxes) do
                waterlevelercore.AddVolume(b.mins.x, b.mins.y, b.mins.z, b.maxs.x, b.maxs.y, b.maxs.z, b.liquid);
            end
        end

        syncVolumeEntities();
    end);

    -- The Water Volume tool picks the materials of the top face and of the
    -- bottom and sides. Any material path works, for example
    -- nature/water_movingplane or models/shadertest/predator. A name that does
    -- not resolve falls back to a translucent blue so the volume never disappears.
    local fallback = Material("color"); -- plain white, tinted by the color below
    local cache = {};
    local unlitCount = 0;
    -- World textures (LightmappedGeneric etc.) need lightmap data a dynamic
    -- mesh does not have, which renders as black faces and blocky garbage.
    -- Only shaders that work on plain meshes are used as they are (VertexLitGeneric
    -- too, e.g. models/shadertest/*: its envmap, bump map and alpha masks are
    -- authored for that shader, and re-wrapping it showed them as raw texture
    -- alpha: pink with holes); anything
    -- else is re-wrapped as UnlitGeneric using the same base texture.
    local passthrough = {
        refract = true,
        unlitgeneric = true,
        spritecard = true,
        vertexlitgeneric = true
    };

    -- A re-wrapped material keeps every parameter the original sets ($basetexture,
    -- $nocull, $refractamount, ...); the values written here are only the defaults for
    -- what the original leaves out (and what the shader needs to work on a mesh).
    local function inherit(m, params)
        for key in pairs(params) do
            if key ~= "Proxies" then
                local own = m:GetString(key);
                if own and own ~= "" then params[key] = own; end
            end
        end
        return params;
    end

    -- World water drawn by the cheap path (nature/water_movingplane): a LightmappedGeneric with an
    -- animated, translucent base texture and a bump map whose alpha masks an envmap. That is what a
    -- map brush shows, so a VertexLitGeneric (lightmaps do not exist on a mesh) with the same
    -- parameters is used, with the original's proxies.
    local carried = {"$envmap", "$envmaptint", "$envmapcontrast", "$normalmapalphaenvmapmask", "$color", "$alpha"};
    local envStrength = CreateClientConVar("waterleveler_envmap_strength", "0.2", true, false, "Reflection strength of world water volumes", 0, 4);
    local envContrast = CreateClientConVar("waterleveler_envmap_contrast", "0.9", true, false, "Reflection contrast of world water volumes (0 = normal, 1 = high)", 0, 1);
    local function wrapLightmappedWater(m, base, bump)
        unlitCount = unlitCount + 1;
        local params = inherit(m, {
            ["$basetexture"] = base,
            ["$bumpmap"] = bump,
            ["$translucent"] = 1,
            ["$bumpframe"] = 0,
            ["$frame"] = 0
        });

        for _, key in ipairs(carried) do
            local own = m:GetString(key);
            if own and own ~= "" then params[key] = own; end
        end

        -- The loaded material reports no $envmap (GetString and GetTexture cannot see it), but its
        -- brush reflects the sky: the .vmt has env_cubemap, masked by the bump map's alpha.
        params["$envmap"] = params["$envmap"] or "env_cubemap";
        params["$normalmapalphaenvmapmask"] = params["$normalmapalphaenvmapmask"] or 1;
        -- 21 frames per second, like the original's proxies (the bump map's frame is set from Lua: a
        -- LinearRamp proxy counts past the last frame, which the engine clamps, so it stood still).
        params.Proxies = {
            AnimatedTexture = {
                animatedtexturevar = "$basetexture",
                animatedtextureframenumvar = "$frame",
                animatedtextureframerate = 21
            }
        };

        local baseTex, bumpTex = m:GetTexture("$basetexture"), m:GetTexture("$bumpmap");
        local baseFrames = baseTex and baseTex:GetNumAnimationFrames() or 1;
        local bumpFrames = bumpTex and bumpTex:GetNumAnimationFrames() or 1;
        local wrapped = CreateMaterial("wl_water_lit_" .. unlitCount, "VertexLitGeneric", params);
        return {
            mat = wrapped,
            color = color_white,
            vcol = false,
            directional = true,
            cull = true, -- single-sided: the inside is drawn with the $bottommaterial (see boxFaces)
            sided = true,
            info = {
                baseFrames = baseFrames,
                bumpFrames = bumpFrames
            },
            -- The same frames the proxies give, set from Lua as well in case a proxy does not run
            -- for a material drawn like this.
            animate = function(t)
                -- Tunable while looking at it: the brush's reflection is sparse and contrasty, a
                -- plain envmap on a mesh is a flat wash.
                local strength = envStrength:GetFloat();
                wrapped:SetVector("$envmaptint", Vector(strength, strength, strength));
                wrapped:SetFloat("$envmapcontrast", envContrast:GetFloat());
                if baseFrames > 1 then wrapped:SetInt("$frame", math.floor(t * 21) % baseFrames); end
                if bumpFrames > 1 then wrapped:SetInt("$bumpframe", math.floor(t * 21) % bumpFrames); end
            end
        };
    end

    -- Any other shader (world textures etc.) is re-wrapped as UnlitGeneric with the same base texture,
    -- and the animation its proxies would have given it is redone in Lua.
    local function wrapGeneric(m)
        local tex = m:GetTexture("$basetexture");
        if tex and not tex:IsError() then
            unlitCount = unlitCount + 1;
            local animate;
            -- The animation does not come along with the texture: re-create the
            -- proxies water materials use. The frame advances like the original
            -- (about 20 per second); the DX7 water also scrolls and scales its
            -- texture ($texscale is how it is recognised) and has a $color.
            local params = {
                ["$basetexture"] = tex:GetName(),
                ["$frame"] = 0,
                ["$vertexcolor"] = 1,
                ["$vertexalpha"] = 1,
                ["$translucent"] = 1,
                ["$nocull"] = 1,
            };

            local scale = m:GetString("$texscale");
            local scaled = scale and scale ~= "";
            if scaled then params["$basetexturetransform"] = "center .5 .5 scale 1 1 rotate 0 translate 0 0"; end
            local um = CreateMaterial("wl_unlit_" .. unlitCount, "UnlitGeneric", inherit(m, params));
            local frames = tex:GetNumAnimationFrames();
            local sv = scaled and m:GetVector("$texscale") or Vector(1, 1, 0);
            animate = function(t)
                if frames > 1 then um:SetInt("$frame", math.floor(t * 20) % frames); end
                if scaled then
                    local a = math.rad(45);
                    local m2 = Matrix();
                    m2:SetScale(Vector(sv.x, sv.y, 1));
                    m2:SetTranslation(Vector(math.cos(a) * 0.025 * t, math.sin(a) * 0.025 * t, 0));
                    um:SetMatrix("$basetexturetransform", m2);
                end
            end;

            local color = Color(255, 255, 255, 200);
            local tintText = m:GetString("$color");
            if tintText and tintText ~= "" then
                local tint = m:GetVector("$color");
                if tint then color = Color(tint.x * 255, tint.y * 255, tint.z * 255, 200); end
            end
            return {
                mat = um,
                color = color,
                vcol = true,
                animate = animate
            };
        end
        return {
            mat = fallback,
            color = Color(40, 120, 220, 90),
            vcol = false
        };
    end

    local function mat(name)
        local entry = cache[name];
        if entry == nil then
            -- "smooth mips noclamp" only applies to image files (png/jpg). VMT
            -- materials get their filtering and mipmaps from their VTF, so
            -- they are loaded as-is and the draw call forces filtering below.
            local lower = name:lower();
            local isImage = lower:match("%.png$") or lower:match("%.jpe?g$");
            local m = isImage and Material(name, "smooth mips noclamp") or Material(name);
            if m:IsError() then
                entry = {
                    mat = fallback,
                    color = Color(40, 120, 220, 90),
                    vcol = false
                };
            else
                -- GetShader() reports the DX level too ("Refract_DX90", "Water_DX90").
                local shader = (m:GetShader() or ""):lower():gsub("_dx%d+$", "");
                -- Texture parameters are read as strings: GetTexture() warns
                -- ("not a texture value") for vars a shader does not declare.
                local nm, dudv = m:GetString("$normalmap"), m:GetString("$dudvmap");
                local bump = m:GetString("$bumpmap");
                if shader == "water" and nm and nm ~= "" then
                    -- The Water shader draws its reflection and refraction only for the
                    -- world's own water surfaces; on a mesh the animated normal map
                    -- (dev/water_normal, advanced by $bumpframe) never shows. A Refract
                    -- material with the same normal map and the same animation does.
                    unlitCount = unlitCount + 1;
                    local params = {
                        ["$normalmap"] = nm,
                        ["$bumpframe"] = 0,
                        ["$refractamount"] = 0.1,
                        ["$bluramount"] = 0,
                        ["$scale"] = "[1 1]",
                        ["$nocull"] = 1,
                        ["$refracttint"] = "{190 215 255}",
                    };

                    local dm = m:GetString("$bumpmap");
                    if dm and dm ~= "" then params["$dudvmap"] = dm; end
                    params.Proxies = {
                        AnimatedTexture = {
                            animatedtexturevar = "$normalmap",
                            animatedtextureframenumvar = "$bumpframe",
                            animatedtextureframerate = 30
                        },
                        -- water_13 and the other Water materials scroll their bump map.
                        TextureScroll = {
                            texturescrollvar = "$bumptransform",
                            texturescrollrate = 0.02,
                            texturescrollangle = 25
                        },
                    };

                    entry = {
                        mat = CreateMaterial("wl_water_" .. unlitCount, "Refract", inherit(m, params)),
                        color = color_white,
                        vcol = false,
                        refract = true
                    };
                elseif shader == "refract" and m:GetInt("$model") == 1 and nm and nm ~= "" then
                    -- Refract materials flagged "$model 1" (models/shadertest/shader3, ...)
                    -- are built for model vertex data and an env_cubemap; on a plain
                    -- mesh they come out pink with holes. Same refraction, minus those.
                    unlitCount = unlitCount + 1;
                    local params = {
                        ["$normalmap"] = nm,
                        ["$refractamount"] = m:GetFloat("$refractamount") or 0.25,
                        ["$bluramount"] = m:GetInt("$bluramount") or 1,
                        ["$scale"] = "[1 1]",
                        ["$nocull"] = 1,
                        Proxies = {
                            TextureScroll = {
                                texturescrollvar = "$bumptransform",
                                texturescrollrate = 0.3,
                                texturescrollangle = -90
                            }
                        },
                    };

                    if dudv and dudv ~= "" then params["$dudvmap"] = dudv; end
                    local tint = m:GetVector("$refracttint");
                    if tint then params["$refracttint"] = ("{%d %d %d}"):format(tint.x * 255, tint.y * 255, tint.z * 255); end
                    entry = {
                        mat = CreateMaterial("wl_refract_" .. unlitCount, "Refract", inherit(m, params)),
                        color = color_white,
                        vcol = false,
                        refract = true
                    };
                elseif shader == "lightmappedgeneric" and bump and bump ~= "" and (m:GetString("$basetexture") or "") ~= "" then
                    entry = wrapLightmappedWater(m, m:GetString("$basetexture"), bump);
                elseif not passthrough[shader] and bump and bump ~= "" then
                    -- World water such as nature/water_movingplane: its look is the animated
                    -- bump map (dev/water_normal), which only a refracting shader can show.
                    unlitCount = unlitCount + 1;
                    entry = {
                        mat = CreateMaterial("wl_bump_" .. unlitCount, "Refract", inherit(m, {
                            ["$normalmap"] = bump,
                            ["$bumpframe"] = 0,
                            ["$refractamount"] = 0.1,
                            ["$bluramount"] = 0,
                            ["$scale"] = "[1 1]",
                            ["$nocull"] = 1,
                            ["$refracttint"] = "{190 215 255}",
                            Proxies = {
                                AnimatedTexture = {
                                    animatedtexturevar = "$normalmap",
                                    animatedtextureframenumvar = "$bumpframe",
                                    animatedtextureframerate = 21
                                }
                            },
                        })),
                        color = color_white,
                        vcol = false,
                        refract = true
                    };
                elseif passthrough[shader] then
                    -- UnlitGeneric (png/jpg) takes its alpha from the vertex colour.
                    entry = {
                        mat = m,
                        color = color_white,
                        vcol = shader == "unlitgeneric",
                        refract = shader == "refract"
                    };
                else
                    entry = wrapGeneric(m);
                end
            end

            -- The material water uses when seen from inside (a map brush's $bottommaterial).
            -- GetString cannot see $bottommaterial of a loaded material (compile-time only), so the
            -- naming convention is used when it comes back empty: <material>_beneath.
            local beneath = not m:IsError() and m:GetString("$bottommaterial");
            if not isImage and not m:IsError() and (not beneath or beneath == "") then
                local guess = name .. "_beneath";
                if not Material(guess):IsError() then beneath = guess; end
            end

            if beneath and beneath ~= "" then entry.beneath = beneath; end
            cache[name] = entry;
        end
        return entry;
    end

    concommand.Add("waterleveler_material_info", function(_, _, args)
        local name = args[1];
        if not name or name == "" then
            local cv = GetConVar("waterleveler_material");
            name = cv and cv:GetString() or "";
            if name == "" then name = defaultMaterial; end
        end

        local original = Material(name);
        local e = mat(name);
        -- GetString returns nothing at all for a parameter the material does not have.
        local function param(m, key)
            return tostring(m:GetString(key) or "(not set)");
        end

        print(("[wl] %s: shader %s, wrapped as %s"):format(name, original:GetShader(), e.mat:GetShader()));
        for _, key in ipairs({"$basetexture", "$bumpmap", "$envmap", "$normalmapalphaenvmapmask", "$translucent", "$frame", "$bumpframe"}) do
            print(("[wl]   %s: original %s | wrapper %s"):format(key, param(original, key), param(e.mat, key)));
        end

        print(("[wl]   flags as integers: $translucent %s, $nocull %s, $normalmapalphaenvmapmask %s"):format(tostring(original:GetInt("$translucent") or "(not set)"), tostring(original:GetInt("$nocull") or "(not set)"), tostring(original:GetInt("$normalmapalphaenvmapmask") or "(not set)")));
        if e.info then print(("[wl]   animation frames: base texture %d, bump map %d"):format(e.info.baseFrames, e.info.bumpFrames)); end
        local function frames(label)
            print(("[wl]   %s: wrapper $frame = %s, $bumpframe = %s"):format(label, param(e.mat, "$frame"), param(e.mat, "$bumpframe")));
        end

        frames("now");
        timer.Simple(0.5, function() frames("after 0.5 s"); end);
        timer.Simple(1, function() frames("after 1 s"); end);
    end);

    local tile = 128; -- world units per texture repeat
    -- One tiled quad, a -> b -> c -> d. Every vertex carries a colour, a normal
    -- and tangents: leaving them unset makes lit or shader-driven materials
    -- (including water) flicker or render black.
    local function emitQuad(a, b, c, d, uLen, vLen, cr, cg, cb, ca, outward)
        local u, v = uLen / tile, vLen / tile;
        local s = (b - a):GetNormalized();
        local t = (d - a):GetNormalized();
        local n = s:Cross(t);
        local verts = {{a, 0, 0}, {b, u, 0}, {c, u, v}, {d, 0, v}};
        if outward then
            -- Culled materials: the engine's front face is the clockwise one,
            -- so wind the quad clockwise as seen from outside the box.
            if n:Dot(outward) > 0 then verts = {verts[4], verts[3], verts[2], verts[1]}; end
            n = outward; -- lighting normal points out of the box
        end

        for _, vert in ipairs(verts) do
            mesh.Position(vert[1]);
            mesh.Color(cr, cg, cb, ca);
            mesh.Normal(n);
            mesh.TangentS(s);
            mesh.TangentT(t);
            -- Bump-mapped VertexLitGeneric reads its tangent from the user data (S, then the
            -- sign of the binormal); without it the bump map has no effect on a mesh.
            mesh.UserData(s.x, s.y, s.z, 1);
            mesh.TexCoord(0, vert[2], vert[3]);
            mesh.AdvanceVertex();
        end
    end

    -- A face is a quad, plus a second one facing the other way for culled materials
    -- (their inside would otherwise be invisible).
    local function emitFace(entry, a, b, c, d, uLen, vLen, outward)
        local r, g, b2, a2 = 255, 255, 255, 255;
        if entry.vcol then r, g, b2, a2 = entry.color.r, entry.color.g, entry.color.b, entry.color.a; end
        if not entry.cull then outward = nil; end
        emitQuad(a, b, c, d, uLen, vLen, r, g, b2, a2, outward);
        if outward and not entry.oneSided then emitQuad(a, b, c, d, uLen, vLen, r, g, b2, a2, -outward); end
    end

    local function setFaceState(entry)
        render.SetMaterial(entry.mat);
        render.CullMode(entry.cull and MATERIAL_CULLMODE_CCW or MATERIAL_CULLMODE_NONE);
        local col = entry.color;
        if entry.vcol then
            -- Colour and alpha travel in the vertices.
            render.SetColorModulation(1, 1, 1);
            render.SetBlend(1);
        else
            render.SetColorModulation(col.r / 255, col.g / 255, col.b / 255);
            render.SetBlend(col.a / 255);
        end
    end

    -- A copy of a material entry that draws only the outward quad of a face.
    local oneSidedCache = setmetatable({}, {
        __mode = "k"
    });

    local function oneSided(entry)
        local copy = oneSidedCache[entry];
        if not copy then
            copy = setmetatable({
                cull = true,
                oneSided = true
            }, {
                __index = entry
            });

            oneSidedCache[entry] = copy;
        end
        return copy;
    end

    local function boxFaces(bx, top, body)
        -- Inset the bottom and sides so they are not coplanar with the
        -- floor or walls the volume rests against (z-fighting).
        local e = 0.5;
        local mn = Vector(bx.mins.x + e, bx.mins.y + e, bx.mins.z + e);
        local mx = Vector(bx.maxs.x - e, bx.maxs.y - e, bx.maxs.z);
        local dx, dy, dz = mx.x - mn.x, mx.y - mn.y, mx.z - mn.z;
        local p000, p100 = Vector(mn.x, mn.y, mn.z), Vector(mx.x, mn.y, mn.z);
        local p110, p010 = Vector(mx.x, mx.y, mn.z), Vector(mn.x, mx.y, mn.z);
        local p001, p101 = Vector(mn.x, mn.y, mx.z), Vector(mx.x, mn.y, mx.z);
        local p111, p011 = Vector(mx.x, mx.y, mx.z), Vector(mn.x, mx.y, mx.z);
        local surface = {
            entry = top,
            a = p001,
            b = p101,
            c = p111,
            d = p011,
            uLen = dx,
            vLen = dy,
            outward = Vector(0, 0, 1)
        };

        -- Sides and bottom first, surface last, so the surface blends over them.
        local faces = {
            {
                entry = body, -- bottom
                a = p000,
                b = p100,
                c = p110,
                d = p010,
                uLen = dx,
                vLen = dy,
                outward = Vector(0, 0, -1)
            },
            {
                entry = body, -- -Y side
                a = p000,
                b = p100,
                c = p101,
                d = p001,
                uLen = dx,
                vLen = dz,
                outward = Vector(0, -1, 0)
            },
            {
                entry = body, -- +Y side
                a = p010,
                b = p110,
                c = p111,
                d = p011,
                uLen = dx,
                vLen = dz,
                outward = Vector(0, 1, 0)
            },
            {
                entry = body, -- -X side
                a = p000,
                b = p010,
                c = p011,
                d = p001,
                uLen = dy,
                vLen = dz,
                outward = Vector(-1, 0, 0)
            },
            {
                entry = body, -- +X side
                a = p100,
                b = p110,
                c = p111,
                d = p101,
                uLen = dy,
                vLen = dz,
                outward = Vector(1, 0, 0)
            },
            surface,
        };

        if not (bx.beneath ~= false and top.sided and top.beneath) then return faces; end
        -- Single-sided water: the outside of every face shows the water, the inside shows its
        -- $bottommaterial, like the two sides of a brush.
        local under = oneSided(mat(top.beneath));
        local both = {};
        for _, f in ipairs(faces) do
            both[#both + 1] = {
                entry = under,
                a = f.a,
                b = f.b,
                c = f.c,
                d = f.d,
                uLen = f.uLen,
                vLen = f.vLen,
                outward = -f.outward
            };

            both[#both + 1] = {
                entry = oneSided(f.entry),
                a = f.a,
                b = f.b,
                c = f.c,
                d = f.d,
                uLen = f.uLen,
                vLen = f.vLen,
                outward = f.outward
            };
        end
        return both;
    end

    local function destroyBuilt(bx)
        if bx.built then
            for _, f in ipairs(bx.built.list) do
                if f.mesh and f.mesh.Destroy then f.mesh:Destroy(); end
            end

            bx.built = nil;
        end
    end

    -- Each volume is drawn by a client-side entity in the engine's translucent
    -- sort, so it is ordered against splashes, sprites and shards by distance
    -- like any other translucent thing (origin = the box centre).
    -- A tiny invisible model, drawn to make the engine bind the local cubemap of a position.
    local probe;
    local function bindCubemapAt(pos)
        if not IsValid(probe) then
            probe = ClientsideModel("models/hunter/blocks/cube025x025x025.mdl", RENDERGROUP_OTHER);
            if not IsValid(probe) then return; end
            probe:SetNoDraw(true);
            probe:SetModelScale(0.01);
        end

        probe:SetPos(pos);
        probe:DrawModel();
    end

    local preview = {}; -- the tool's preview volume: the mesh of its current size, around the origin
    local function drawVolume(bx)
        local top = mat(bx.material ~= "" and bx.material or defaultMaterial);
        local body = top;
        local now = CurTime();
        if top.animate then top.animate(now); end
        if top.directional then
            -- env_cubemap is whichever cubemap the engine bound last, which changes with what is in
            -- view (the reflection brightened and dimmed as most of the volume left the screen).
            -- Drawing a model at the volume's centre makes the engine bind the cubemap for that spot.
            bindCubemapAt((bx.mins + bx.maxs) * 0.5);
            -- A bump map only shows under uneven light (a brush gets the map's lightmap): light
            -- from above, dimmer from the sides.
            render.ResetModelLighting(0.45, 0.45, 0.45);
            -- Local lights are left over from whichever model was drawn last (they change with
            -- the view); only the ambient light set here should count.
            render.SetLocalModelLights({});
            render.SetModelLighting(BOX_TOP, 1, 1, 1);
            render.SetModelLighting(BOX_BOTTOM, 0.2, 0.2, 0.2);
        else
            render.ResetModelLighting(1, 1, 1); -- VertexLitGeneric materials: even, full light
        end

        -- Refract materials sample a copy of the frame, which is only updated when
        -- asked to: once per volume, before any face, so no face refracts another one
        -- (that flickered), and never stale.
        if top.refract then render.UpdateRefractTexture(); end
        -- Smooth filtering with mipmaps (anisotropic = trilinear + aniso).
        render.PushFilterMag(TEXFILTER.ANISOTROPIC);
        render.PushFilterMin(TEXFILTER.ANISOTROPIC);
        -- Every volume is drawn from static meshes. Rebuilding them as immediate meshes every
        -- frame shares the dynamic vertex buffer with the particles and sprites a gunshot
        -- creates, which corrupted the faces (and made the preview flicker). The preview moves
        -- with the aim point, so its mesh is built around the origin and translated.
        local owner = bx;
        local moved = false;
        if not bx.persist then
            local centre = (bx.mins + bx.maxs) * 0.5;
            owner = preview;
            owner.mins = bx.mins - centre;
            owner.maxs = bx.maxs - centre;
            owner.material = bx.material;
            owner.beneath = bx.beneath;
            local translation = Matrix();
            translation:SetTranslation(centre);
            cam.PushModelMatrix(translation);
            moved = true;
        end

        local key = (owner.material or "") .. tostring(owner.maxs - owner.mins) .. (owner.beneath ~= false and "/beneath" or "");
        if not owner.built or owner.built.key ~= key then
            destroyBuilt(owner);
            local list = {};
            for _, f in ipairs(boxFaces(owner, top, body)) do
                local m = Mesh(f.entry.mat);
                mesh.Begin(m, MATERIAL_QUADS, f.entry.cull and not f.entry.oneSided and 2 or 1);
                emitFace(f.entry, f.a, f.b, f.c, f.d, f.uLen, f.vLen, f.outward);
                mesh.End();
                list[#list + 1] = {
                    entry = f.entry,
                    mesh = m
                };
            end

            owner.built = {
                key = key,
                list = list
            };
        end

        for _, f in ipairs(owner.built.list) do
            setFaceState(f.entry);
            f.mesh:Draw();
        end

        if moved then cam.PopModelMatrix(); end
        render.PopFilterMin();
        render.PopFilterMag();
        render.CullMode(MATERIAL_CULLMODE_CCW);
        render.SetColorModulation(1, 1, 1);
        render.SetBlend(1);
    end

    -- Also used by the Water Volume tool for its preview.
    WLWater.DrawVolume = drawVolume;
    scripted_ents.Register({
        Type = "anim",
        Base = "base_anim",
        Spawnable = false,
        RenderGroup = RENDERGROUP_TRANSLUCENT,
        Initialize = function(self) self:SetRenderMode(RENDERMODE_TRANSALPHA); end,
        Draw = function(self) if self.WLBox then drawVolume(self.WLBox); end end,
    }, "wl_volume_draw");

    local volumeEnts = {};
    syncVolumeEntities = function()
        for _, e in ipairs(volumeEnts) do
            if IsValid(e) then
                if e.WLBox then destroyBuilt(e.WLBox); end
                e:Remove();
            end
        end

        volumeEnts = {};
        for _, bx in ipairs(boxes) do
            local e = ents.CreateClientside("wl_volume_draw");
            if IsValid(e) then
                local centre = (bx.mins + bx.maxs) * 0.5;
                e:SetModel("models/error.mdl");
                e:SetPos(centre);
                e:Spawn();
                bx.persist = true;
                e.WLBox = bx;
                e:SetRenderBounds(bx.mins - centre, bx.maxs - centre);
                volumeEnts[#volumeEnts + 1] = e;
            end
        end
    end;

    -- Map cleanups remove client-side entities too.
    hook.Add("PostCleanupMap", "wl_volumes_entities", function() syncVolumeEntities(); end);
    -- Debugging: what do the client's own water queries answer?
    local function volumeAt(p)
        for i, b in ipairs(boxes) do
            if p.x >= b.mins.x and p.x <= b.maxs.x and p.y >= b.mins.y and p.y <= b.maxs.y and p.z >= b.mins.z and p.z <= b.maxs.z then return i, b.maxs.z - p.z; end
        end
    end

    local function describe()
        local ply = LocalPlayer();
        if not IsValid(ply) then return "no player"; end
        local eye = ply:EyePos();
        local idx, depth = volumeAt(eye);
        local contents = util.PointContents(eye);
        local line = ("eye %s | in volume: %s | eye depth below its surface: %s | util.PointContents(eye) = 0x%x (water %s, slime %s) | WaterLevel() = %d"):format(tostring(eye), idx and "#" .. idx or "no", depth and ("%.1f"):format(depth) or "-", contents, tostring(bit.band(contents, CONTENTS_WATER) ~= 0), tostring(bit.band(contents, CONTENTS_SLIME) ~= 0), ply:WaterLevel());
        if waterlevelercore then line = line .. (" | module point-in-volume: %s"):format(tostring(waterlevelercore.PointInVolume(eye.x, eye.y, eye.z))); end
        return line;
    end

    concommand.Add("waterleveler_client_status", function()
        print("[wl] map: " .. game.GetMap());
        if not waterlevelercore then return print("[wl] client module not loaded (gmcl_waterlevelercore_win64.dll missing or failed): the client's water queries do not see volumes"); end
        print("[wl] " .. waterlevelercore.HookStatus());
        print(("[wl] contents hook: %s | volumes: %d (script knows %d) | point queries answered with water: %d | traces stopped at the surface: %d"):format(tostring(waterlevelercore.IsContentsHooked()), waterlevelercore.VolumeCount(), #boxes, waterlevelercore.ContentsHits(), waterlevelercore.TraceHits()));
        for i, b in ipairs(boxes) do
            print(("[wl] volume #%d %s .. %s (%s) depth %.0f"):format(i, tostring(b.mins), tostring(b.maxs), b.liquid, b.maxs.z - b.mins.z));
        end

        print("[wl] " .. describe());
    end);

    local watching, lastContents, lastTraces = false, 0, 0;
    concommand.Add("waterleveler_watch", function(_, _, args)
        watching = args[1] ~= nil and args[1] ~= "0" or args[1] == nil and not watching;
        if not watching then
            timer.Remove("wl_watch");
            return print("[wl] watch off");
        end

        print("[wl] watch on: each line shows how many point queries and traces the engine made against the volumes during the last second");
        lastContents = waterlevelercore and waterlevelercore.ContentsHits() or 0;
        lastTraces = waterlevelercore and waterlevelercore.TraceHits() or 0;
        timer.Create("wl_watch", 1, 0, function()
            local c = waterlevelercore and waterlevelercore.ContentsHits() or 0;
            local t = waterlevelercore and waterlevelercore.TraceHits() or 0;
            print(("[wl] +%d point queries, +%d traces | %s"):format(c - lastContents, t - lastTraces, describe()));
            lastContents, lastTraces = c, t;
        end);
    end);

    -- Fully submerged (water level 3): the engine's water screen overlay.
    hook.Add("RenderScreenspaceEffects", "wl_volumes_overlay", function()
        local ply = LocalPlayer();
        if not IsValid(ply) or not ply:Alive() then return; end
        if ply:WaterLevel() >= 3 then DrawMaterialOverlay("effects/water_warp01", 0.1); end
    end);
end
