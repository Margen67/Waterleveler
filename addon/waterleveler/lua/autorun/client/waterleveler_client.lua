-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Margen67
-- Client half of the addon:
--  * loads the client module (gmcl_waterlevelercore) when it is installed: it lets the
--    client's own water queries (swimming prediction, splashes, the underwater explosion
--    sound) see the volumes;
--  * adds "Use with Water Volume tool" to the right-click menu of material icons in the
--    spawnmenu (the stock menu and Extended Spawnmenu's), next to "Use with Material Tool".
pcall(require, "waterlevelercore");
local wrapped = {}; -- content type functions this file made, so they are not wrapped twice
local function useWithWaterTool(spawnname)
    RunConsoleCommand("waterleveler_material", spawnname);
    spawnmenu.ActivateTool("waterleveler");
    surface.PlaySound("garrysmod/ui_click.wav");
end

-- The icon's OpenMenu builds its DermaMenu itself, so the menu is caught while it is
-- built: once the "Use with Material Tool" option is added, ours follows it.
local function wrapOpenMenu(icon, spawnname)
    local openMenu = icon.OpenMenu;
    if not openMenu then return; end
    icon.OpenMenu = function(...)
        local dermaMenu = DermaMenu;
        DermaMenu = function(...)
            DermaMenu = dermaMenu;
            local menu = dermaMenu(...);
            local addOption = menu.AddOption;
            local added = false;
            menu.AddOption = function(self, name, ...)
                local option = addOption(self, name, ...);
                if not added and isstring(name) and name:find("Material Tool", 1, true) then
                    added = true;
                    addOption(self, "Use with Water Volume tool", function() useWithWaterTool(spawnname); end):SetIcon("icon16/wrench.png");
                end
                return option;
            end;
            return menu;
        end;

        local ok, err = pcall(openMenu, ...);
        DermaMenu = dermaMenu;
        if not ok then ErrorNoHalt(err, "\n"); end
    end;
end

local function wrapMaterialContentType()
    local original = spawnmenu.GetContentType("material");
    if not original or wrapped[original] then return; end
    local function create(container, obj)
        local icon = original(container, obj);
        if IsValid(icon) and obj and obj.spawnname then wrapOpenMenu(icon, obj.spawnname); end
        return icon;
    end

    wrapped[create] = true;
    spawnmenu.AddContentType("material", create);
end

wrapMaterialContentType();
-- Addons that load after this file and register their own "material" type replace it.
hook.Add("InitPostEntity", "waterleveler_material_menu", wrapMaterialContentType);
hook.Add("PopulateContent", "waterleveler_material_menu", wrapMaterialContentType);
