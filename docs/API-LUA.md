# Atlas Code Lua API (v0, experimental)

Atlas is customized with Lua 5.4, like Neovim. **This API may still change**: it will be frozen and
versioned once it has been tested with real hands.

## Where your code goes

- **`init.lua`** in the workspace root (`/sdcard/Android/data/com.atlascode.xr/files/`).
  It is loaded at startup and on `:reload`. The first time, Atlas creates a fully commented template.
- **Scripts**: any `.lua` file. Open it and type `:run`: whatever is in the buffer is executed
  (no need to save). It runs after `init.lua`.
- `:reload` / `:source` reloads `init.lua` and the last script from scratch (hot reload).

Errors appear in the output panel with file and line (`init.lua, line 3: ...`). A failure in
`init.lua` does not prevent the script from running.

## Sandbox

A script cannot touch files (`io`), the system (`os`), load code (`load`, `require`, `dofile`) or use
`debug`. It has an instruction cap per call (400,000), 16 MB of memory, 64 orbs and 64 functions
registered with `gesture.on`. `math`, `string`, `table` and `utf8` are available.

## Functions

| | |
|---|---|
| `print(...)` | Writes to the output panel. |
| `orb(x, y, z)` | Creates an orb. `x` to your right, `y` up, `-z` ahead, in meters relative to you. |
| `orbs()` | List of the script's orbs. |
| Orb: `o.x o.y o.z o.radius o.intensity o.pulse o.speed o.id` | Readable and writable (with limits). |
| Orb: `o:color(r,g,b)`, `o:rim(r,g,b)` | Core and rim color, 0 to 1. |
| `buffer.text()` `buffer.file()` `buffer.lines()` `buffer.line(n)` | The open file, read-only. |
| `editor.mode()` `editor.cursor()` | Mode and position (line, column), 1-based. |
| `editor.message(text)` | Shows a notice. |
| `editor.command(text)` | Runs a `:` command (Normal mode only; no git, no `:run`, no quit). |
| `gesture.on(event, function)` | Reacts to an event (below). |
| `function on_frame(t, dt)` | If you define it, it is called every frame. |

## `gesture.on` events

| Event | Arguments |
|---|---|
| `drawer_open`, `drawer_close` | |
| `editor_open`, `editor_close` | |
| `card_tap` | file name, 1 if it is a folder |
| `orb_thrown` | speed (m/s) |
| `wall_hit` | speed (m/s) |
| `environment` | environment name |
| `desk_mapped`, `wall_mapped` | |

## Configuration: `config`

The defaults are already tuned. `config.name = value` changes them (numbers are clamped to a sensible
range). Removing the line from `init.lua` and reloading restores the default.

| Value | Default | What it is |
|---|---|---|
| `fan_radius` | 0.62 | Distance (m) from you to the card file that opens on your desk, behind the keyboard (0.3 to 1.2). |
| `card_width` | 0.21 | Width (m) of a card (0.10 to 0.6). |
| `card_release` | `"return"` | When a moved card is dropped: `"return"` sends it back, `"stay"` leaves it. |
| `keyboard` | `"own"` | Virtual keyboard for typing without a physical one: `"own"` is the flat keyboard with a trackpad on your desk, `"system"` is Meta's floating system keyboard. |
| `panel_width` | 1.2 | Width (m) of the editor panel (0.6 to 2.4). Wider means bigger text at the same distance. |
| `panel_distance` | 0.9 | Distance (m) at which the editor panel appears (0.5 to 2.0). The headset's focal plane is roughly 1 to 2 m, but a panel farther than your arm's reach cannot be grabbed with the fist. |
| `font_scale` | 1.0 | Multiplier for the editor text size (0.6 to 2.0). |
| `gravity` | 4.5 | Gravity (m/s²) of the thrown orb (0 to 20). |
| `throw_min_speed` | 0.8 | Minimum hand speed (m/s) for a release to count as a throw. |

## Example

```lua
config.card_release = "stay"      -- cards stay where you drop them
config.fan_radius = 0.6

gesture.on("editor_open", function()
  editor.message("editing " .. buffer.file())
end)

local o = orb(0, -0.1, -0.5)
o:color(1, 0.5, 0.2)
function on_frame(t, dt) o.y = -0.1 + 0.03 * math.sin(t) end
```

## Not yet available

`panel` (create your own panels), remapping gestures, writing to the buffer, per-plugin permissions and
loading plugins from other repositories. They are on the roadmap.
