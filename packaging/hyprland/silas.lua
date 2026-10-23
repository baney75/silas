-- Silas keys for Hyprland's Lua config (Omarchy and others). The Arch package installs this
-- as /usr/share/silas/hyprland.lua. Load it from ~/.config/hypr/bindings.lua:
--
--   local silas = dofile("/usr/share/silas/hyprland.lua")
--   silas.hold("SUPER + SHIFT + code:201")  -- optional: a laptop's Copilot key
--
-- Loading it puts Silas on the keys Omarchy gives dictation: hold F9 to talk, SUPER + CTRL + X
-- to start and stop hands-free. Hold a key, speak, release, and Silas types; tap it for
-- hands-free. Hyprland swallows the keys, so apps never see them.

local silas = {}

local function bind(keys, description, command, options)
  if o and o.bind then
    o.bind(keys, description, command, options)
  else
    local opts = options or {}
    opts.description = description
    hl.bind(keys, hl.dsp.exec_cmd(command), opts)
  end
end

-- Hold to talk, release to type. Replaces whatever the key did before.
function silas.hold(keys)
  hl.unbind(keys)
  bind(keys, "Dictate with Silas (hold)", "silas-ctl press")
  bind(keys, "Dictate with Silas (release)", "silas-ctl release", { release = true })
end

-- Press once to start hands-free, again to type. It fires when the keys come up, so Silas
-- never types while the chord's modifiers are still held.
function silas.toggle(keys)
  hl.unbind(keys)
  bind(keys, "Dictate with Silas (hands-free)", "silas-ctl toggle", { release = true })
end

silas.hold("F9")
silas.toggle("SUPER + CTRL + X")

return silas
