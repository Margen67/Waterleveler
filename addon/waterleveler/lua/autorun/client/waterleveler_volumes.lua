-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- The client half of the Water Volume script (drawing, materials,
-- the debug commands). Shared parts: autorun/waterleveler_volumes.lua.
-- The server sends the volumes (the "wl_volumes" net message); each one is drawn as a
-- translucent box with its material, and one without a material is not drawn at all.
-- The engine side of the volumes on this client is the client module (waterlevelercore),
-- loaded by autorun/client/waterleveler.lua.
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
        if name == "" then
            print("[wl] no material is set (a volume without one is not drawn)");
            return;
        end
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

-- The six faces of the box from mn to mx; the surface (top) is last so it blends over the rest.
local function shell(mn, mx, top, body)
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
    return {
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
end

local function boxFaces(bx, top, body)
    -- The outer layer is the collision box, except that its bottom starts ContactDepth up
    -- (the box reaches that far into the floor it rests on). Only the inner layer (the
    -- _beneath faces seen from inside) is inset further, from the bottom and sides, so it
    -- does not z-fight with the floor or walls the volume rests against.
    local outer = shell(Vector(bx.mins.x, bx.mins.y, bx.mins.z + (WLWater.ContactDepth or 0)), bx.maxs, top, body);
    if not (bx.beneath ~= false and top.sided and top.beneath) then return outer; end
    -- Single-sided water: the outside of every face shows the water, the inside shows its
    -- $bottommaterial, like the two sides of a brush.
    local under = oneSided(mat(top.beneath));
    local e = 0.5;
    local inner = shell(Vector(bx.mins.x + e, bx.mins.y + e, bx.mins.z + e), Vector(bx.maxs.x - e, bx.maxs.y - e, bx.maxs.z), top, body);
    local both = {};
    for _, f in ipairs(inner) do
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
    end

    for _, f in ipairs(outer) do
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
local preview = {}; -- the tool's preview volume: the mesh of its current size, around the origin
local function drawVolume(bx)
    -- A volume without a material has no mesh at all.
    if not bx.material or bx.material == "" then return; end
    local top = mat(bx.material);
    local body = top;
    local now = CurTime();
    if top.animate then top.animate(now); end
    if top.directional then
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

    -- The meshes are rebuilt only when the material, the size or the _beneath option
    -- changed: compared as plain numbers, so a frame allocates nothing for the check.
    -- Rounded to 0.01: the preview's size is a difference of positions that move with the
    -- aim point, and float noise must not rebuild its meshes every frame.
    local sizeX, sizeY, sizeZ = math.floor((owner.maxs.x - owner.mins.x) * 100 + 0.5), math.floor((owner.maxs.y - owner.mins.y) * 100 + 0.5), math.floor((owner.maxs.z - owner.mins.z) * 100 + 0.5);
    local material, beneath = owner.material or "", owner.beneath ~= false;
    local built = owner.built;
    if not built or built.material ~= material or built.beneath ~= beneath or built.sizeX ~= sizeX or built.sizeY ~= sizeY or built.sizeZ ~= sizeZ then
        destroyBuilt(owner);
        -- Neighbouring faces that share a material go into one mesh (one draw call). The
        -- faces stay in the same order, so the surface still blends over the sides.
        local list = {};
        local run, quads;
        local function flush()
            if not run then return; end
            local m = Mesh(run[1].entry.mat);
            mesh.Begin(m, MATERIAL_QUADS, quads);
            for _, f in ipairs(run) do
                emitFace(f.entry, f.a, f.b, f.c, f.d, f.uLen, f.vLen, f.outward);
            end

            mesh.End();
            list[#list + 1] = {
                entry = run[1].entry,
                mesh = m
            };

            run = nil;
        end

        for _, f in ipairs(boxFaces(owner, top, body)) do
            if run and run[1].entry ~= f.entry then flush(); end
            if not run then run, quads = {}, 0; end
            run[#run + 1] = f;
            quads = quads + (f.entry.cull and not f.entry.oneSided and 2 or 1);
        end

        flush();
        owner.built = {
            material = material,
            beneath = beneath,
            sizeX = sizeX,
            sizeY = sizeY,
            sizeZ = sizeZ,
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
        -- No material: nothing to draw, so no entity either.
        local e = bx.material ~= "" and ents.CreateClientside("wl_volume_draw") or nil;
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

-- Fully submerged (water level 3): the water screen warp. It is drawn here, by this script
-- (the engine does not draw one for these volumes); the tool can turn it off.
local warpEnabled = CreateClientConVar("waterleveler_underwater_warp", "1", true, false, "0 removes the underwater screen warp", 0, 1);
hook.Add("RenderScreenspaceEffects", "wl_volumes_overlay", function()
    if not warpEnabled:GetBool() then return; end
    local ply = LocalPlayer();
    if not IsValid(ply) or not ply:Alive() then return; end
    if ply:WaterLevel() >= 3 then DrawMaterialOverlay("effects/water_warp01", 0.1); end
end);
