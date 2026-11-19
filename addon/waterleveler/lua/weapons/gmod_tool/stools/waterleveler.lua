-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- Water Volume tool (Tools tab > Construction).
--   Left click: create a water volume where you are looking. It rests against the
--               surface you aim at and can be moved up or down with Offset.
-- Needs the waterleveler addon's autorun script (WLWater) and the binary module.

TOOL.Category = "Construction"
TOOL.Name = "#tool.waterleveler.name"

TOOL.ClientConVar = {
    sizex = "128",
    sizey = "128",
    sizez = "128",
    offset = "0",
    liquid = "water", -- "water" or "slime": the contents and the fluid's physics material
    fluid = "1",
    fluid_damping = "0.01",
    fluid_torque = "0",
    fluid_viscosity = "0",
    fluid_aero = "0",
    fluid_currentx = "0",
    fluid_currenty = "0",
    fluid_currentz = "0",
    material = "nature/water_movingplane",
}

local MIN_SIZE, MAX_SIZE = 1, 16384
local MAX_FLUID, MAX_CURRENT = 10, 1000

-- The numeric convars have limits: the console can't set a value outside them.
-- (Created here, before the tool registers TOOL.ClientConVar, which would
-- otherwise create them without limits.)
if CLIENT then
    local limits = {
        sizex = { 128, MIN_SIZE, MAX_SIZE }, sizey = { 128, MIN_SIZE, MAX_SIZE }, sizez = { 128, MIN_SIZE, MAX_SIZE },
        offset = { 0, -MAX_SIZE, MAX_SIZE },
        fluid = { 1, 0, 1 },
        fluid_damping = { 0.01, 0, MAX_FLUID }, fluid_torque = { 0, 0, MAX_FLUID },
        fluid_viscosity = { 0, 0, MAX_FLUID }, fluid_aero = { 0, 0, 1 },
        fluid_currentx = { 0, -MAX_CURRENT, MAX_CURRENT }, fluid_currenty = { 0, -MAX_CURRENT, MAX_CURRENT },
        fluid_currentz = { 0, -MAX_CURRENT, MAX_CURRENT },
    }
    for name, l in pairs(limits) do
        CreateClientConVar("waterleveler_" .. name, tostring(l[1]), true, true, "", l[2], l[3])
    end
end

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
    language.Add("tool.waterleveler.name", "Water Volume")
    language.Add("tool.waterleveler.desc", "Create a water volume that behaves like engine water")
    language.Add("tool.waterleveler.left", "Create a water volume")
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
        liquid = self:GetClientInfo("liquid"):lower() == "slime" and "slime" or "water",
        fluid = self:GetClientNumber("fluid", 1) ~= 0,
        fluidOptions = {
            damping = math.Clamp(self:GetClientNumber("fluid_damping", 0.01), 0, MAX_FLUID),
            torque = math.Clamp(self:GetClientNumber("fluid_torque", 0), 0, MAX_FLUID),
            viscosity = math.Clamp(self:GetClientNumber("fluid_viscosity", 0), 0, MAX_FLUID),
            aerodynamics = self:GetClientNumber("fluid_aero", 0) ~= 0,
            currentx = math.Clamp(self:GetClientNumber("fluid_currentx", 0), -MAX_CURRENT, MAX_CURRENT),
            currenty = math.Clamp(self:GetClientNumber("fluid_currenty", 0), -MAX_CURRENT, MAX_CURRENT),
            currentz = math.Clamp(self:GetClientNumber("fluid_currentz", 0), -MAX_CURRENT, MAX_CURRENT),
        },
        material = self:GetClientInfo("material"),
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
        "nature/water_movingplane_dx70",
        "gm_construct/water_13",
        "models/shadertest/predator",
        "models/shadertest/shader3",
    }

    function TOOL.BuildCPanel(panel)
        panel:Help("Creates a water volume where you aim. Swimming, water level and impact splashes work like normal water.")

        local sliders = {}
        local offset

        local function updateOffsetRange()
            if not IsValid(offset) then return end
            local max = math.max(
                GetConVar("waterleveler_sizex"):GetFloat(),
                GetConVar("waterleveler_sizey"):GetFloat(),
                GetConVar("waterleveler_sizez"):GetFloat())
            offset:SetMin(-max)
            offset:SetMax(max)
            local cur = GetConVar("waterleveler_offset"):GetFloat()
            if math.abs(cur) > max then RunConsoleCommand("waterleveler_offset", tostring(math.Clamp(cur, -max, max))) end
        end

        -- Sets all three axes at once.
        local all = panel:NumSlider("All axes", "", MIN_SIZE, MAX_SIZE, 0)
        all:SetValue(math.max(
            GetConVar("waterleveler_sizex"):GetFloat(),
            GetConVar("waterleveler_sizey"):GetFloat(),
            GetConVar("waterleveler_sizez"):GetFloat()))
        all.OnValueChanged = function(_, value)
            value = math.Round(value)
            for _, axis in ipairs({ "x", "y", "z" }) do
                if GetConVar("waterleveler_size" .. axis):GetInt() ~= value then
                    RunConsoleCommand("waterleveler_size" .. axis, tostring(value))
                end
            end
            timer.Simple(0, updateOffsetRange)
        end

        for _, axis in ipairs({ { "x", "Width (X)" }, { "y", "Length (Y)" }, { "z", "Height (Z)" } }) do
            local sl = panel:NumSlider(axis[2], "waterleveler_size" .. axis[1], MIN_SIZE, MAX_SIZE, 0)
            sl.OnValueChanged = function() timer.Simple(0, updateOffsetRange) end
            sliders[#sliders + 1] = sl
        end

        offset = panel:NumSlider("Offset", "waterleveler_offset", -MAX_SIZE, MAX_SIZE, 0)
        panel:ControlHelp("Moves the volume up (or down) from where it would rest. The limit is the biggest size above.")
        panel:ControlHelp("Underwater sound muffling needs a depth of about 132 units (131-132 is the limit).")
        updateOffsetRange()

        panel:Help("Liquid (also the fluid's physics material)")
        local liquid = panel:ComboBox("")
        liquid:SetSortItems(false)
        local current = GetConVar("waterleveler_liquid")
        local isSlime = current and current:GetString() == "slime"
        liquid:AddChoice("Water", "water", not isSlime)
        liquid:AddChoice("Slime", "slime", isSlime)
        liquid.OnSelect = function(_, _, _, data) RunConsoleCommand("waterleveler_liquid", data) end
        panel:ControlHelp("Slime: slime-coloured splashes and ripples, no wading, no water explosion effect.")
        panel:CheckBox("Buoyancy (props float)", "waterleveler_fluid")
        panel:ControlHelp("The values below are the engine's fluid settings. The defaults are those of a map's water.")
        panel:NumSlider("Damping (drag)", "waterleveler_fluid_damping", 0, MAX_FLUID, 3)
        panel:NumSlider("Torque factor", "waterleveler_fluid_torque", 0, MAX_FLUID, 3)
        panel:NumSlider("Viscosity factor", "waterleveler_fluid_viscosity", 0, MAX_FLUID, 3)
        panel:CheckBox("Aerodynamic drag", "waterleveler_fluid_aero")
        panel:NumSlider("Current X", "waterleveler_fluid_currentx", -MAX_CURRENT, MAX_CURRENT, 0)
        panel:NumSlider("Current Y", "waterleveler_fluid_currenty", -MAX_CURRENT, MAX_CURRENT, 0)
        panel:NumSlider("Current Z", "waterleveler_fluid_currentz", -MAX_CURRENT, MAX_CURRENT, 0)

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
        materialRow("Material", "waterleveler_material")
    end

    -- Preview: the volume drawn with the selected materials, plus an outline, where the player is looking.
    local function convar(name, default)
        local cv = GetConVar(name)
        return cv and cv:GetFloat() or default
    end

    -- The preview is hidden while the toolgun is being drawn or holstered, and while a prop is held.
    local function toolgunBusy(ply)
        local vm = ply:GetViewModel(0)
        if not IsValid(vm) then return false end
        local act = vm:GetSequenceActivity(vm:GetSequence())
        return act == ACT_VM_HOLSTER or (act == ACT_VM_DRAW and vm:GetCycle() < 0.99)
    end

    local function holdingProp(ply, tr)
        local used = ply:GetEntityInUse()
        if IsValid(used) and used ~= ply and used:GetClass() ~= "worldspawn" then return true end
        local ent = tr.Entity
        return IsValid(ent) and ent.IsPlayerHolding ~= nil and ent:IsPlayerHolding()
    end

    local function convarString(name, default)
        local cv = GetConVar(name)
        return cv and cv:GetString() or default
    end

    hook.Add("PostDrawTranslucentRenderables", "waterleveler_tool_preview", function(_, skybox)
        if skybox then return end
        local ply = LocalPlayer()
        local wep = IsValid(ply) and ply:GetActiveWeapon()
        if not IsValid(wep) or wep:GetClass() ~= "gmod_tool" or wep:GetMode() ~= "waterleveler" then return end
        if not WLWater or toolgunBusy(ply) then return end

        local tr = ply:GetEyeTrace()
        if not tr.Hit or holdingProp(ply, tr) then return end

        local size = readSize(function(name) return convar("waterleveler_" .. name, 128) end)
        local mins, maxs = WLWater.Placement(tr, size, readOffset(function(name) return convar("waterleveler_" .. name, 0) end, size))

        if WLWater.DrawVolume then
            local function str(name, default)
                local cv = GetConVar(name)
                return cv and cv:GetString() or default
            end
            WLWater.DrawVolume({
                mins = mins,
                maxs = maxs,
                material = str("waterleveler_material", ""),
            })
        end
        local slime = convarString("waterleveler_liquid", "water"):lower() == "slime"
        local r, g, b = 90, 190, 255
        if slime then r, g, b = 90, 255, 120 end
        render.DrawWireframeBox(Vector(0, 0, 0), Angle(0, 0, 0), mins, maxs, Color(r, g, b, 255), true)
        render.DrawWireframeBox(Vector(0, 0, 0), Angle(0, 0, 0), mins, maxs, Color(r, g, b, 60), false)
    end)
end
