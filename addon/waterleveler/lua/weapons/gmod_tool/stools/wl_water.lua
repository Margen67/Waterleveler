-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- Water Volume tool (Tools tab > Construction).
--   Left click: create a water volume where you are looking. It rests against the
--               surface you aim at and can be moved up or down with Offset.
-- Needs the waterleveler addon's autorun script (WLWater) and the binary module.

TOOL.Category = "Construction"
TOOL.Name = "#tool.wl_water.name"

TOOL.ClientConVar = {
    sizex = "128",
    sizey = "128",
    sizez = "128",
    offset = "0",
    fluid = "1",
    surface = "nature/water_movingplane",
    body = "nature/water_movingplane",
}

local MIN_SIZE, MAX_SIZE = 16, 2048

local function readSize(get)
    return Vector(
        math.Clamp(get("sizex"), MIN_SIZE, MAX_SIZE),
        math.Clamp(get("sizey"), MIN_SIZE, MAX_SIZE),
        math.Clamp(get("sizez"), MIN_SIZE, MAX_SIZE))
end

-- The offset is limited to the biggest dimension of the volume.
local function readOffset(get, size)
    local max = math.max(size.x, size.y, size.z)
    return math.Clamp(get("offset"), -max, max)
end

if CLIENT then
    language.Add("tool.wl_water.name", "Water Volume")
    language.Add("tool.wl_water.desc", "Create a water volume that behaves like engine water")
    language.Add("tool.wl_water.left", "Create a water volume")
    language.Add("Undone_Water volume", "Undone water volume")
end

TOOL.Information = { { name = "left" } }

function TOOL:LeftClick(trace)
    if not trace.Hit then return false end
    if CLIENT then return true end

    local ply = self:GetOwner()
    if not WLWater or not WLWater.Spawn then
        ply:ChatPrint("[waterleveler] the water module is not loaded")
        return false
    end
    if not WLWater.Allowed(ply) then
        ply:ChatPrint("[waterleveler] only superadmins can create water volumes")
        return false
    end

    local function get(name) return self:GetClientNumber(name, 0) end
    local size = readSize(get)
    local mins, maxs = WLWater.Placement(trace, size, readOffset(get, size))
    local id, err = WLWater.Spawn(ply, mins, maxs, {
        fluid = self:GetClientNumber("fluid", 1) ~= 0,
        surface = self:GetClientInfo("surface"),
        body = self:GetClientInfo("body"),
    })
    if not id then
        ply:ChatPrint("[waterleveler] could not create the volume: " .. tostring(err))
        return false
    end
    return true
end

if CLIENT then
    local PRESETS = {
        "nature/water_movingplane",
        "nature/water_canals/canal_wall",
        "models/shadertest/shader3",
    }

    function TOOL.BuildCPanel(panel)
        panel:Help("Creates a water volume where you aim. Swimming, water level and impact splashes work like normal water.")

        local sliders = {}
        local offset

        local function updateOffsetRange()
            if not IsValid(offset) then return end
            local max = math.max(
                GetConVar("wl_water_sizex"):GetFloat(),
                GetConVar("wl_water_sizey"):GetFloat(),
                GetConVar("wl_water_sizez"):GetFloat())
            offset:SetMin(-max)
            offset:SetMax(max)
            local cur = GetConVar("wl_water_offset"):GetFloat()
            if math.abs(cur) > max then RunConsoleCommand("wl_water_offset", tostring(math.Clamp(cur, -max, max))) end
        end

        for _, axis in ipairs({ { "x", "Width (X)" }, { "y", "Length (Y)" }, { "z", "Height (Z)" } }) do
            local sl = panel:NumSlider(axis[2], "wl_water_size" .. axis[1], MIN_SIZE, MAX_SIZE, 0)
            sl.OnValueChanged = function() timer.Simple(0, updateOffsetRange) end
            sliders[#sliders + 1] = sl
        end

        offset = panel:NumSlider("Offset", "wl_water_offset", -MAX_SIZE, MAX_SIZE, 0)
        panel:ControlHelp("Moves the volume up (or down) from where it would rest. The limit is the biggest size above.")
        updateOffsetRange()

        panel:CheckBox("Buoyancy (props float)", "wl_water_fluid")

        local function materialRow(label, convar)
            panel:Help(label)
            local entry = panel:TextEntry("", convar)
            local combo = panel:ComboBox("")
            combo:SetSortItems(false)
            combo:AddChoice("Presets...", nil, true)
            for _, m in ipairs(PRESETS) do combo:AddChoice(m) end
            combo.OnSelect = function(_, _, value)
                if value ~= "Presets..." then RunConsoleCommand(convar, value) end
            end
            return entry
        end
        materialRow("Surface material (top face)", "wl_water_surface")
        materialRow("Body material (bottom and sides)", "wl_water_body")
    end

    -- Preview: an outline of the volume where the player is looking, with the surface face tinted.
    local function convar(name, default)
        local cv = GetConVar(name)
        return cv and cv:GetFloat() or default
    end

    hook.Add("PostDrawTranslucentRenderables", "wl_water_tool_preview", function(_, skybox)
        if skybox then return end
        local ply = LocalPlayer()
        local wep = IsValid(ply) and ply:GetActiveWeapon()
        if not IsValid(wep) or wep:GetClass() ~= "gmod_tool" or wep:GetMode() ~= "wl_water" then return end
        if not WLWater then return end

        local tr = ply:GetEyeTrace()
        if not tr.Hit then return end

        local size = readSize(function(name) return convar("wl_water_" .. name, 128) end)
        local mins, maxs = WLWater.Placement(tr, size, readOffset(function(name) return convar("wl_water_" .. name, 0) end, size))

        render.SetColorMaterial()
        render.DrawBox(Vector(0, 0, 0), Angle(0, 0, 0), Vector(mins.x, mins.y, maxs.z - 0.5), Vector(maxs.x, maxs.y, maxs.z), Color(60, 150, 255, 70))
        render.DrawWireframeBox(Vector(0, 0, 0), Angle(0, 0, 0), mins, maxs, Color(90, 190, 255, 255), true)
        render.DrawWireframeBox(Vector(0, 0, 0), Angle(0, 0, 0), mins, maxs, Color(90, 190, 255, 60), false)
    end)
end
