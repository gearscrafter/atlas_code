-- Atlas Code init.lua: your configuration. Loaded at startup and with :reload.
-- Everything below is commented out: remove the "--" from what you want to change.
-- The defaults are already tuned; this only overrides them.

-- Distance (m) from you to the card file that opens on your desk, and the width of a card.
-- config.fan_radius = 0.62
-- config.card_width = 0.21

-- What happens when you drop a card you moved: "return" (goes back) or "stay" (stays put).
-- config.card_release = "return"

-- Virtual keyboard: "own" (flat on your desk, with a trackpad) or "system" (Meta's floating keyboard).
-- config.keyboard = "own"

-- Reading comfort. You can also try values live with :font 1.2, :panel 1.4 and :distance 1.1 (or +0.1 / -0.1
-- to nudge them), then copy the ones you like here.
-- config.font_scale = 1.0       -- text size multiplier (0.6 to 2.0)
-- config.panel_width = 1.2      -- editor panel width in meters (0.6 to 2.4)
-- config.panel_distance = 0.9   -- how far in front of you the editor appears (0.5 to 2.0)

-- Environment orb: gravity (m/s²) and minimum hand speed (m/s) for a release to count as a throw.
-- config.gravity = 4.5
-- config.throw_min_speed = 0.8

-- React to what you do in space. Events: drawer_open, drawer_close, editor_open,
-- editor_close, card_tap(name, is_folder), orb_thrown(speed), wall_hit(speed),
-- environment(name), desk_mapped, wall_mapped.
-- gesture.on("editor_open", function() editor.message("editor opened on " .. buffer.file()) end)

-- Your own orbs: orb(x, y, z) with x to your right, y up, -z ahead.
-- local o = orb(0.0, -0.1, -0.5)
-- o:color(1.0, 0.5, 0.2)
-- function on_frame(t, dt) o.y = -0.1 + 0.03 * math.sin(t) end
