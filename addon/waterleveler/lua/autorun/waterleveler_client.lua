-- Loads the client half of the module (gmcl_waterlevelercore) when it is installed.
-- It makes the water explosion's sound play at the blast instead of on the surface.
if CLIENT then
    pcall(require, "waterlevelercore")
end
