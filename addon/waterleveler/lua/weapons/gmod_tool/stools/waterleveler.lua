-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- Water Volume tool (Tools tab > Construction).
--   Left click: create a water volume where you are looking. It rests against the
--               surface you aim at and can be moved up or down with Offset.
--   Reload:     remove the water volumes that touch the preview.
-- Needs the waterleveler addon's autorun script (WLWater) and the binary module.
cleanup.Register("waterleveler");
TOOL.Category = "Construction";
TOOL.Name = "#tool.waterleveler.name";
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
    beneath = "0", -- the inside shows the material's _beneath texture; off: the material, double-sided
    color_r = "90", -- the liquid's colour: blue for water, green for slime (reset when the liquid changes)
    color_g = "190",
    color_b = "255",
    color_a = "110",
    outline = "0", -- draw the box's edges in outline_*
    outline_r = "90",
    outline_g = "190",
    outline_b = "255",
    outline_a = "255",
};

local defaults = TOOL.ClientConVar; -- the menu's reset button needs them after TOOL is gone
local minSize, maxSize = 1, 32768;
-- The material that means "no material: one translucent colour" (color_*).
local solidMaterial = "Solid color";
local function isSolid(material)
    return string.lower(string.Trim(material)) == string.lower(solidMaterial);
end

local maxFluid, maxCurrent = 10, 1000;
-- The numeric convars have limits: the console can't set a value outside them.
-- (Created here, before the tool registers TOOL.ClientConVar, which would
-- otherwise create them without limits.)
if CLIENT then
    local limits = {
        sizex = {128, minSize, maxSize},
        sizey = {128, minSize, maxSize},
        sizez = {128, minSize, maxSize},
        offset = {0, -maxSize, maxSize},
        fluid = {1, 0, 1},
        fluid_damping = {0.01, 0, maxFluid},
        fluid_torque = {0, 0, maxFluid},
        fluid_viscosity = {0, 0, maxFluid},
        fluid_aero = {0, 0, 1},
        fluid_currentx = {0, -maxCurrent, maxCurrent},
        fluid_currenty = {0, -maxCurrent, maxCurrent},
        fluid_currentz = {0, -maxCurrent, maxCurrent},
        color_r = {90, 0, 255},
        color_g = {190, 0, 255},
        color_b = {255, 0, 255},
        color_a = {110, 0, 255},
        outline = {0, 0, 1},
        outline_r = {90, 0, 255},
        outline_g = {190, 0, 255},
        outline_b = {255, 0, 255},
        outline_a = {255, 0, 255},
    };

    for name, l in pairs(limits) do
        CreateClientConVar("waterleveler_" .. name, tostring(l[1]), true, true, "", l[2], l[3]);
    end
end

local function readSize(get)
    return Vector(math.Clamp(get("sizex"), minSize, maxSize), math.Clamp(get("sizey"), minSize, maxSize), math.Clamp(get("sizez"), minSize, maxSize));
end

-- The offset is limited to the biggest dimension of the volume.
local function readOffset(get, size)
    local max = math.max(size.x, size.y, size.z);
    return math.Clamp(get("offset"), -max, max);
end

-- The colour stored in the convars <prefix>_r, _g, _b and _a.
local function readColor(get, prefix)
    local function byte(channel)
        return math.Clamp(math.floor(get(prefix .. "_" .. channel)), 0, 255);
    end
    return Color(byte("r"), byte("g"), byte("b"), byte("a"));
end

if CLIENT then
    language.Add("tool.waterleveler.name", "Water Volume");
    language.Add("tool.waterleveler.desc", "Create a water volume that behaves like real water");
    language.Add("tool.waterleveler.left", "Create a water volume");
    language.Add("tool.waterleveler.reload", "Remove the water volumes that touch the preview");
    language.Add("Undone_Water Volume", "Undone Water Volume");
    language.Add("Cleanup_waterleveler", "Water volumes");
    language.Add("Cleaned_waterleveler", "Cleaned up all water volumes");
end

TOOL.Information = {
    {
        name = "left"
    },
    {
        name = "reload"
    }
};

function TOOL:Reload(trace)
    if not trace.Hit then return false; end
    if CLIENT then return true; end
    local ply = self:GetOwner();
    if not WLWater or not WLWater.RemoveInBox then
        ply:ChatPrint("[waterleveler] the water module is not loaded");
        return false;
    end

    if not WLWater.Allowed(ply) then
        ply:ChatPrint("[waterleveler] only superadmins can remove water volumes");
        return false;
    end

    local function get(name)
        return self:GetClientNumber(name, 0);
    end

    local size = readSize(get);
    local mins, maxs = WLWater.Placement(trace, size, readOffset(get, size));
    return WLWater.RemoveInBox(mins, maxs) > 0;
end

function TOOL:LeftClick(trace)
    if not trace.Hit then return false; end
    if CLIENT then return true; end
    local ply = self:GetOwner();
    if not WLWater or not WLWater.Spawn then
        ply:ChatPrint("[waterleveler] the water module is not loaded");
        return false;
    end

    if not WLWater.Allowed(ply) then
        ply:ChatPrint("[waterleveler] only superadmins can create water volumes");
        return false;
    end

    local function get(name)
        return self:GetClientNumber(name, 0);
    end

    local size = readSize(get);
    local mins, maxs = WLWater.Placement(trace, size, readOffset(get, size));
    local id, err = WLWater.Spawn(ply, mins, maxs, {
        liquid = self:GetClientInfo("liquid"):lower() == "slime" and "slime" or "water",
        fluid = self:GetClientNumber("fluid", 1) ~= 0,
        fluidOptions = {
            damping = math.Clamp(self:GetClientNumber("fluid_damping", 0.01), 0, maxFluid),
            torque = math.Clamp(self:GetClientNumber("fluid_torque", 0), 0, maxFluid),
            viscosity = math.Clamp(self:GetClientNumber("fluid_viscosity", 0), 0, maxFluid),
            aerodynamics = self:GetClientNumber("fluid_aero", 0) ~= 0,
            currentx = math.Clamp(self:GetClientNumber("fluid_currentx", 0), -maxCurrent, maxCurrent),
            currenty = math.Clamp(self:GetClientNumber("fluid_currenty", 0), -maxCurrent, maxCurrent),
            currentz = math.Clamp(self:GetClientNumber("fluid_currentz", 0), -maxCurrent, maxCurrent),
        },
        material = isSolid(self:GetClientInfo("material")) and "" or self:GetClientInfo("material"),
        beneath = self:GetClientNumber("beneath", 0) ~= 0,
        solid = isSolid(self:GetClientInfo("material")),
        color = readColor(get, "color"),
        outline = self:GetClientNumber("outline", 0) ~= 0,
        outlineColor = readColor(get, "outline"),
    });

    if not id then
        ply:ChatPrint("[waterleveler] could not create the volume: " .. tostring(err));
        return false;
    end
    return true;
end

if CLIENT then
    local presets = {"nature/water_movingplane", "nature/water_movingplane_dx70", "gm_construct/water_13", "models/shadertest/predator", "models/shadertest/shader3",};
    local rowHeight = 20;
    ---@param panel ControlPanel
    function TOOL.BuildCPanel(panel)
        local function numSlider(label, convar, min, max, decimals)
            local sl = panel:NumSlider(label, convar, min, max, decimals);
            sl:SetTall(rowHeight);
            return sl;
        end

        -- A control whose visibility follows a convar; its form row goes with it.
        local toggled = {};
        local function showWhen(ctrl, visible)
            toggled[#toggled + 1] = {
                ctrl = ctrl,
                visible = visible
            };
        end

        local function refresh()
            if not IsValid(panel) then return; end
            for _, t in ipairs(toggled) do
                local row = t.ctrl:GetParent();
                if IsValid(row) and row ~= panel then row:SetVisible(t.visible()); end
            end

            panel:InvalidateLayout(true);
        end

        local function flag(convar)
            return GetConVar("waterleveler_" .. convar):GetBool();
        end

        local function colorMixer(prefix)
            local mixer = vgui.Create("DColorMixer");
            mixer:SetPalette(false);
            mixer:SetAlphaBar(true);
            mixer:SetTall(92);
            for _, channel in ipairs({"R", "G", "B", "A"}) do
                mixer["SetConVar" .. channel](mixer, "waterleveler_" .. prefix .. "_" .. channel:lower());
            end

            panel:AddItem(mixer);
            return mixer;
        end

        local sliders = {};
        local offset;
        local function updateOffsetRange()
            if not IsValid(offset) then return; end
            local max = math.max(GetConVar("waterleveler_sizex"):GetFloat(), GetConVar("waterleveler_sizey"):GetFloat(), GetConVar("waterleveler_sizez"):GetFloat());
            offset:SetMin(-max);
            offset:SetMax(max);
            local cur = GetConVar("waterleveler_offset"):GetFloat();
            if math.abs(cur) > max then RunConsoleCommand("waterleveler_offset", tostring(math.Clamp(cur, -max, max))); end
        end

        -- Sets all three axes at once.
        local all = numSlider("All axes", "", minSize, maxSize, 0);
        all:SetValue(math.max(GetConVar("waterleveler_sizex"):GetFloat(), GetConVar("waterleveler_sizey"):GetFloat(), GetConVar("waterleveler_sizez"):GetFloat()));
        ---@diagnostic disable-next-line: inject-field
        all.OnValueChanged = function(_, value)
            value = math.Round(value);
            for _, axis in ipairs({"x", "y", "z"}) do
                if GetConVar("waterleveler_size" .. axis):GetInt() ~= value then RunConsoleCommand("waterleveler_size" .. axis, tostring(value)); end
            end

            timer.Simple(0, updateOffsetRange);
        end;

        for _, axis in ipairs({{"x", "Width (X)"}, {"y", "Length (Y)"}, {"z", "Height (Z)"}}) do
            local sl = numSlider(axis[2], "waterleveler_size" .. axis[1], minSize, maxSize, 0);
            ---@diagnostic disable-next-line: inject-field
            sl.OnValueChanged = function() timer.Simple(0, updateOffsetRange); end;
            sliders[#sliders + 1] = sl;
        end

        offset = numSlider("Offset", "waterleveler_offset", -maxSize, maxSize, 0);
        panel:ControlHelp("Moves the volume up (or down) from where it would rest. The limit is the biggest size above.");
        panel:ControlHelp("The muffled underwater explosion sound needs the blast to be more than 128 units below the surface (the client looks up to 512).");
        updateOffsetRange();
        local liquid = panel:ComboBox("Liquid");
        liquid:SetSortItems(false);
        liquid:SetTall(rowHeight);
        local current = GetConVar("waterleveler_liquid");
        local isSlime = current and current:GetString() == "slime";
        liquid:AddChoice("Water", "water", not isSlime);
        liquid:AddChoice("Slime", "slime", isSlime);
        ---@diagnostic disable-next-line: inject-field
        liquid.OnSelect = function(_, _, _, data)
            RunConsoleCommand("waterleveler_liquid", data);
            -- The colours follow the liquid: blue for water, green for slime.
            local r, g, b = WLWater.LiquidColor(data);
            for prefix, alpha in pairs({
                color = WLWater.FillAlpha,
                outline = 255
            }) do
                RunConsoleCommand("waterleveler_" .. prefix .. "_r", tostring(r));
                RunConsoleCommand("waterleveler_" .. prefix .. "_g", tostring(g));
                RunConsoleCommand("waterleveler_" .. prefix .. "_b", tostring(b));
                RunConsoleCommand("waterleveler_" .. prefix .. "_a", tostring(alpha));
            end
        end;

        panel:CheckBox("Underwater screen warp", "waterleveler_underwater_warp");
        panel:CheckBox("Buoyancy (props float)", "waterleveler_fluid");
        numSlider("Damping (drag)", "waterleveler_fluid_damping", 0, maxFluid, 3);
        numSlider("Torque factor", "waterleveler_fluid_torque", 0, maxFluid, 3);
        numSlider("Viscosity factor", "waterleveler_fluid_viscosity", 0, maxFluid, 3);
        panel:CheckBox("Aerodynamic drag", "waterleveler_fluid_aero");
        numSlider("Current X", "waterleveler_fluid_currentx", -maxCurrent, maxCurrent, 0);
        numSlider("Current Y", "waterleveler_fluid_currenty", -maxCurrent, maxCurrent, 0);
        numSlider("Current Z", "waterleveler_fluid_currentz", -maxCurrent, maxCurrent, 0);
        -- Material: one text entry with a button that drops down the presets.
        local box = vgui.Create("DPanel");
        box:SetPaintBackground(false);
        box:SetTall(rowHeight + 4);
        box:Dock(FILL);
        local entry = vgui.Create("DTextEntry", box);
        entry:Dock(FILL);
        entry:SetConVar("waterleveler_material");
        local arrow = vgui.Create("DButton", box);
        arrow:Dock(RIGHT);
        arrow:DockMargin(2, 0, 0, 0);
        arrow:SetWide(rowHeight + 4);
        arrow:SetText("");
        arrow:SetIcon("icon16/bullet_arrow_down.png");
        ---@diagnostic disable-next-line: inject-field
        arrow.DoClick = function()
            local menu = DermaMenu();
            menu:AddOption(solidMaterial, function() RunConsoleCommand("waterleveler_material", solidMaterial); end);
            for _, m in ipairs(presets) do
                menu:AddOption(m, function() RunConsoleCommand("waterleveler_material", m); end);
            end

            menu:Open();
        end;

        local label = vgui.Create("DLabel");
        label:SetText("Material");
        label:SetDark(true);
        panel:AddItem(label, box);
        local function solid()
            return isSolid(GetConVar("waterleveler_material"):GetString());
        end

        showWhen(colorMixer("color"), solid);
        showWhen(panel:CheckBox("Inside shows the material's _beneath texture", "waterleveler_beneath"), function() return not solid(); end);
        local outline = panel:CheckBox("Outline", "waterleveler_outline");
        showWhen(colorMixer("outline"), function() return flag("outline"); end);
        -- Debug (minimized): these only change what this client draws, for every volume, in fixed
        -- colors (blue water, green slime) that ignore the colors above.
        local debugForm = vgui.Create("DForm", panel);
        debugForm:SetLabel("Debug");
        debugForm:SetExpanded(false);
        debugForm:CheckBox("Draw every volume as a solid color (even invisible water)", "waterleveler_debug_fill");
        debugForm:CheckBox("Draw an outline around every volume", "waterleveler_debug_outline");
        debugForm:CheckBox("The outline draws over everything else", "waterleveler_debug_outline_top");
        debugForm:ControlHelp("Blue for water, green for slime. The colors above do not apply.");
        panel:AddItem(debugForm);
        local reset = panel:Button("Reset to defaults");
        ---@diagnostic disable-next-line: inject-field
        reset.DoClick = function()
            for name, default in pairs(defaults) do
                RunConsoleCommand("waterleveler_" .. name, default);
            end

            RunConsoleCommand("waterleveler_underwater_warp", "1");
            liquid:ChooseOptionID(1);
        end;

        for _, convar in ipairs({"material", "outline"}) do
            cvars.AddChangeCallback("waterleveler_" .. convar, function() timer.Simple(0, refresh); end, "waterleveler_panel_" .. convar);
        end

        -- Tighter rows: the form's items start with 10 units of padding above them.
        for _, item in ipairs(panel.Items or {}) do
            if IsValid(item) then item:DockPadding(10, 2, 10, 0); end
        end

        -- A little more room above the Outline checkbox and the Debug section.
        for _, ctrl in ipairs({outline, debugForm}) do
            local row = ctrl:GetParent();
            if IsValid(row) then row:DockPadding(10, 8, 10, 0); end
        end

        refresh();
    end

    -- Preview: the volume drawn with the selected materials, plus an outline, where the player is looking.
    local function convar(name, default)
        local cv = GetConVar(name);
        return cv and cv:GetFloat() or default;
    end

    -- The preview is hidden while the toolgun is being drawn or holstered, and while a prop is held.
    local function toolgunBusy(ply)
        local vm = ply:GetViewModel(0);
        if not IsValid(vm) then return false; end
        local act = vm:GetSequenceActivity(vm:GetSequence());
        return act == ACT_VM_HOLSTER or act == ACT_VM_DRAW and vm:GetCycle() < 0.99;
    end

    ---@param ply Player
    ---@param tr TraceResult
    ---@return boolean
    local function holdingProp(ply, tr)
        local used = ply:GetEntityInUse();
        if IsValid(used) and used ~= ply and used:GetClass() ~= "worldspawn" then return true; end
        local ent = tr.Entity;
        if ent == nil or not IsValid(ent) then return false; end
        return ent.IsPlayerHolding ~= nil and ent:IsPlayerHolding();
    end

    hook.Add("PostDrawTranslucentRenderables", "waterleveler_tool_preview", function(_, skybox)
        if skybox then return; end
        local ply = LocalPlayer();
        if not IsValid(ply) then return; end
        ---@type Weapon|{GetMode: fun(self: Weapon): string}
        local wep = ply:GetActiveWeapon();
        if not IsValid(wep) or wep:GetClass() ~= "gmod_tool" or wep:GetMode() ~= "waterleveler" then return; end
        if not WLWater or toolgunBusy(ply) then return; end
        local tr = ply:GetEyeTrace();
        if not tr.Hit or holdingProp(ply, tr) then return; end
        local size = readSize(function(name) return convar("waterleveler_" .. name, 128); end);
        local mins, maxs = WLWater.Placement(tr, size, readOffset(function(name) return convar("waterleveler_" .. name, 0); end, size));
        if WLWater.DrawVolume then
            local function str(name, default)
                local cv = GetConVar(name);
                return cv and cv:GetString() or default;
            end

            WLWater.DrawVolume({
                mins = mins,
                maxs = maxs,
                material = isSolid(str("waterleveler_material", "")) and "" or str("waterleveler_material", ""),
                beneath = convar("waterleveler_beneath", 0) ~= 0,
                solid = isSolid(str("waterleveler_material", "")),
                color = Color(convar("waterleveler_color_r", 90), convar("waterleveler_color_g", 190), convar("waterleveler_color_b", 255), convar("waterleveler_color_a", 110)),
            });
        end

        -- The guide outline uses the outline color picker's color and is drawn over everything else.
        local r, g, b, a = convar("waterleveler_outline_r", 90), convar("waterleveler_outline_g", 190), convar("waterleveler_outline_b", 255), convar("waterleveler_outline_a", 255);
        cam.IgnoreZ(true);
        render.DrawWireframeBox(Vector(0, 0, 0), Angle(0, 0, 0), mins, maxs, Color(r, g, b, a), false);
        cam.IgnoreZ(false);
    end);
end
