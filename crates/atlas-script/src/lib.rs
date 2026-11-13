//! Atlas Code scripting (Lua 5.4). It is how you customize Atlas: an `init.lua`
//! loaded at startup, and scripts run with `:run`.
//!
//! It only works with data (no XR or Vulkan), so it is tested on the PC. The headset gives it the
//! editor state ([`Snapshot`]), delivers the gesture events ([`Runtime::emit`]) and
//! collects what the script asks for ([`Runtime::take_commands`], [`Runtime::config`], [`Runtime::orbs`]).
//!
//! # Lua API (v0, experimental: it may change until it is frozen)
//! ```text
//! orb(x, y, z) -> Orb          -- an orb relative to you: x right, y up, -z ahead
//! orbs() -> {Orb}              -- the script's orbs;  o:color(r,g,b)  o:rim(r,g,b)  o.x o.y o.z ...
//! print(...)                   -- goes to the output panel
//!
//! buffer.text()  buffer.file()  buffer.lines()  buffer.line(n)   -- the open file (read only)
//! editor.mode()  editor.cursor() -> line, column
//! editor.message("text")       -- shows a notice
//! editor.command("w")          -- runs a `:` command (w, e file, ...)
//!
//! gesture.on("event", function(...) end)    -- react to what you do in space
//! config.fan_radius = 0.5      -- defaults you can change (see `Config`)
//! function on_frame(t, dt) end -- called every frame
//! ```
//!
//! The script runs in a closed box: no files (`io`), no system (`os`), no loading
//! code (`load`, `require`), with a limit on instructions and memory so that an infinite
//! loop or a greedy script cannot freeze the headset.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use mlua::{
    Function, HookTriggers, Lua, LuaOptions, MultiValue, RegistryKey, StdLib, UserData, UserDataFields, UserDataMethods,
    Value, Variadic, VmState,
};

/// Orbs a script can create.
pub const MAX_ORBS: usize = 64;
/// Maximum instructions per run (loading the script, one `on_frame` or one event).
const MAX_INSTRUCTIONS: u64 = 400_000;
/// How often (in instructions) the limit is checked.
const HOOK_EVERY: u32 = 1000;
/// Maximum memory of the script.
const MAX_MEMORY: usize = 16 * 1024 * 1024;
/// Output lines (`print`) kept until they are collected.
const MAX_OUTPUT_LINES: usize = 200;
/// Pending orders a script can pile up between two collections.
const MAX_COMMANDS: usize = 100;
/// Functions a script can register with `gesture.on`.
const MAX_HANDLERS: usize = 64;

/// What happens to a card you drop after moving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardRelease {
    /// It floats back to its place in the fan (the default).
    Return,
    /// Stays where you drop it, like a sheet left on a desk.
    Stay,
}

/// Which virtual keyboard is used when typing without a physical one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardKind {
    /// Atlas's own keyboard, flat on the desk and with a trackpad (the default).
    Own,
    /// Meta's system keyboard (a floating panel).
    System,
}

/// Defaults the user can change from Lua with `config.name = value`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Distance (m) at which the drawer's cards float.
    pub fan_radius: f32,
    /// Width (m) of a card.
    pub card_width: f32,
    pub card_release: CardRelease,
    pub keyboard: KeyboardKind,
    /// Width (m) of the editor panel: wider means bigger text at the same distance.
    pub panel_width: f32,
    /// Distance (m) at which the editor panel appears.
    pub panel_distance: f32,
    /// Multiplier for the editor's text size (1 = the usual one).
    pub font_scale: f32,
    /// Gravity (m/s²) of the thrown orb.
    pub gravity: f32,
    /// Minimum hand speed (m/s) for releasing the orb to count as a throw.
    pub throw_min_speed: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { fan_radius: 0.62, card_width: 0.21, card_release: CardRelease::Return, keyboard: KeyboardKind::Own, panel_width: 1.2, panel_distance: 0.9, font_scale: 1.0, gravity: 4.5, throw_min_speed: 0.8 }
    }
}

impl Config {
    /// Changes a numeric value by its name, within its range. With `relative` it adds `value` to the current one.
    /// Returns the resulting value. The `:font`, `:panel` and `:distance` commands use it.
    pub fn adjust(&mut self, key: &str, value: f32, relative: bool) -> Result<f32, String> {
        if !value.is_finite() {
            return Err(format!("{key} needs a finite number"));
        }
        let (field, min, max) = match key {
            "panel_width" => (&mut self.panel_width, 0.6, 2.4),
            "panel_distance" => (&mut self.panel_distance, 0.5, 2.0),
            "font_scale" => (&mut self.font_scale, 0.6, 2.0),
            other => return Err(format!("{other} cannot be changed this way")),
        };
        *field = if relative { *field + value } else { value }.clamp(min, max);
        Ok(*field)
    }
}

/// Editor state the script sees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    pub file: String,
    pub mode: String,
    /// Cursor line and column, from 1.
    pub cursor: (usize, usize),
}

/// What a script asks the headset for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Show a notice.
    Message(String),
    /// Run a `:` command (without the colon).
    Ex(String),
}

/// An argument of an event.
#[derive(Debug, Clone, PartialEq)]
pub enum EventArg {
    Num(f64),
    Text(String),
}

/// An orb as the script describes it, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScriptOrb {
    pub position: [f32; 3],
    pub radius: f32,
    pub core: [f32; 3],
    pub rim: [f32; 3],
    pub intensity: f32,
    pub pulse: f32,
    pub speed: f32,
    pub phase: f32,
}

impl ScriptOrb {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: [x, y, z],
            radius: 0.04,
            core: [0.30, 0.90, 1.0],
            rim: [0.55, 0.60, 1.0],
            intensity: 1.0,
            pulse: 0.05,
            speed: 1.8,
            phase: 0.0,
        }
    }
}

/// A script error, with the file (`init` or `script`) and the line if known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub chunk: String,
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let place = if self.chunk == "init" { "init.lua, " } else { "" };
        match self.line {
            Some(line) => write!(f, "{place}line {line}: {}", self.message),
            None => write!(f, "{place}{}", self.message),
        }
    }
}

impl ScriptError {
    /// Lua writes errors as `script:LINE: message` (or `init:`), sometimes inside a
    /// call stack. It keeps the first line in that format.
    fn from_lua(e: &mlua::Error, default_chunk: &str) -> Self {
        let text = e.to_string();
        let line_of = |raw: &str| -> Option<(String, usize, String)> {
            for chunk in ["script", "init"] {
                if let Some(rest) = raw.trim().strip_prefix(&format!("{chunk}:")) {
                    let (n, message) = rest.split_once(':')?;
                    return Some((chunk.to_string(), n.trim().parse().ok()?, message.trim().to_string()));
                }
            }
            None
        };
        let clean = |raw: &str| raw.trim().trim_start_matches("runtime error: ").trim_start_matches("syntax error: ").to_string();
        let mut lines = text.lines().map(clean).filter(|l| !l.is_empty());
        let first = lines.next().unwrap_or_else(|| "error".to_string());
        if let Some((chunk, line, message)) = line_of(&first) {
            return Self { chunk, line: Some(line), message };
        }
        let found = lines.find_map(|l| line_of(&l));
        Self { chunk: found.as_ref().map_or(default_chunk.to_string(), |f| f.0.clone()), line: found.map(|f| f.1), message: first }
    }
}

type OrbData = Rc<RefCell<ScriptOrb>>;

/// Handle of an orb inside the script (`local a = orb(...)`).
#[derive(Clone)]
struct OrbHandle {
    id: usize,
    data: OrbData,
}

/// An `f32` as `f64` without dragging the conversion error along (0.52 and not 0.5199999809).
fn short(v: f32) -> f64 {
    format!("{v}").parse().unwrap_or(v as f64)
}

fn clamp01(v: f64) -> f32 {
    (v as f32).clamp(0.0, 1.0)
}

impl UserData for OrbHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, o| Ok(o.id));
        fields.add_field_method_get("x", |_, o| Ok(short(o.data.borrow().position[0])));
        fields.add_field_method_set("x", |_, o, v: f64| Ok(o.data.borrow_mut().position[0] = v as f32));
        fields.add_field_method_get("y", |_, o| Ok(short(o.data.borrow().position[1])));
        fields.add_field_method_set("y", |_, o, v: f64| Ok(o.data.borrow_mut().position[1] = v as f32));
        fields.add_field_method_get("z", |_, o| Ok(short(o.data.borrow().position[2])));
        fields.add_field_method_set("z", |_, o, v: f64| Ok(o.data.borrow_mut().position[2] = v as f32));
        fields.add_field_method_get("radius", |_, o| Ok(short(o.data.borrow().radius)));
        fields.add_field_method_set("radius", |_, o, v: f64| Ok(o.data.borrow_mut().radius = (v as f32).clamp(0.005, 0.5)));
        fields.add_field_method_get("intensity", |_, o| Ok(short(o.data.borrow().intensity)));
        fields.add_field_method_set("intensity", |_, o, v: f64| Ok(o.data.borrow_mut().intensity = (v as f32).clamp(0.0, 4.0)));
        fields.add_field_method_get("pulse", |_, o| Ok(short(o.data.borrow().pulse)));
        fields.add_field_method_set("pulse", |_, o, v: f64| Ok(o.data.borrow_mut().pulse = (v as f32).clamp(0.0, 0.5)));
        fields.add_field_method_get("speed", |_, o| Ok(short(o.data.borrow().speed)));
        fields.add_field_method_set("speed", |_, o, v: f64| Ok(o.data.borrow_mut().speed = v as f32));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("color", |_, o, (r, g, b): (f64, f64, f64)| {
            o.data.borrow_mut().core = [clamp01(r), clamp01(g), clamp01(b)];
            Ok(())
        });
        methods.add_method("rim", |_, o, (r, g, b): (f64, f64, f64)| {
            o.data.borrow_mut().rim = [clamp01(r), clamp01(g), clamp01(b)];
            Ok(())
        });
    }
}

/// The `config` table in Lua: reading and writing the defaults, with reasonable limits.
#[derive(Clone)]
struct ConfigHandle(Rc<RefCell<Config>>);

impl UserData for ConfigHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("fan_radius", |_, c| Ok(short(c.0.borrow().fan_radius)));
        fields.add_field_method_set("fan_radius", |_, c, v: f64| Ok(c.0.borrow_mut().fan_radius = (v as f32).clamp(0.3, 1.2)));
        fields.add_field_method_get("card_width", |_, c| Ok(short(c.0.borrow().card_width)));
        fields.add_field_method_set("card_width", |_, c, v: f64| Ok(c.0.borrow_mut().card_width = (v as f32).clamp(0.10, 0.6)));
        fields.add_field_method_get("gravity", |_, c| Ok(short(c.0.borrow().gravity)));
        fields.add_field_method_set("gravity", |_, c, v: f64| Ok(c.0.borrow_mut().gravity = (v as f32).clamp(0.0, 20.0)));
        fields.add_field_method_get("throw_min_speed", |_, c| Ok(short(c.0.borrow().throw_min_speed)));
        fields.add_field_method_set("throw_min_speed", |_, c, v: f64| Ok(c.0.borrow_mut().throw_min_speed = (v as f32).clamp(0.2, 4.0)));
        fields.add_field_method_get("card_release", |_, c| {
            Ok(match c.0.borrow().card_release {
                CardRelease::Return => "return",
                CardRelease::Stay => "stay",
            })
        });
        fields.add_field_method_set("card_release", |_, c, v: String| {
            c.0.borrow_mut().card_release = match v.as_str() {
                "return" => CardRelease::Return,
                "stay" => CardRelease::Stay,
                other => return Err(mlua::Error::runtime(format!("card_release must be 'return' or 'stay', not '{other}'"))),
            };
            Ok(())
        });
        fields.add_field_method_get("panel_width", |_, c| Ok(short(c.0.borrow().panel_width)));
        fields.add_field_method_set("panel_width", |_, c, v: f64| {
            c.0.borrow_mut().adjust("panel_width", v as f32, false).map(|_| ()).map_err(mlua::Error::runtime)
        });
        fields.add_field_method_get("panel_distance", |_, c| Ok(short(c.0.borrow().panel_distance)));
        fields.add_field_method_set("panel_distance", |_, c, v: f64| {
            c.0.borrow_mut().adjust("panel_distance", v as f32, false).map(|_| ()).map_err(mlua::Error::runtime)
        });
        fields.add_field_method_get("font_scale", |_, c| Ok(short(c.0.borrow().font_scale)));
        fields.add_field_method_set("font_scale", |_, c, v: f64| {
            c.0.borrow_mut().adjust("font_scale", v as f32, false).map(|_| ()).map_err(mlua::Error::runtime)
        });
        fields.add_field_method_get("keyboard", |_, c| {
            Ok(match c.0.borrow().keyboard {
                KeyboardKind::Own => "own",
                KeyboardKind::System => "system",
            })
        });
        fields.add_field_method_set("keyboard", |_, c, v: String| {
            c.0.borrow_mut().keyboard = match v.as_str() {
                "own" => KeyboardKind::Own,
                "system" => KeyboardKind::System,
                other => return Err(mlua::Error::runtime(format!("keyboard must be 'own' or 'system', not '{other}'"))),
            };
            Ok(())
        });
    }
}

type Scene = Rc<RefCell<Vec<OrbHandle>>>;
type Output = Rc<RefCell<Vec<String>>>;
type Handlers = Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>;

pub struct Runtime {
    lua: Option<Lua>,
    scene: Scene,
    output: Output,
    budget: Rc<Cell<u64>>,
    commands: Rc<RefCell<Vec<Command>>>,
    snapshot: Rc<RefCell<Snapshot>>,
    handlers: Handlers,
    config: Rc<RefCell<Config>>,
    init: Option<String>,
    script: Option<String>,
    has_frame: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            lua: None,
            scene: Rc::default(),
            output: Rc::default(),
            budget: Rc::default(),
            commands: Rc::default(),
            snapshot: Rc::default(),
            handlers: Rc::default(),
            config: Rc::new(RefCell::new(Config::default())),
            init: None,
            script: None,
            has_frame: false,
        }
    }

    /// Creates a closed interpreter with the Atlas functions.
    fn sandbox(&self) -> mlua::Result<Lua> {
        // Only math, strings, tables and UTF-8: no io, os, package or debug.
        let lua = Lua::new_with(StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8, LuaOptions::default())?;
        lua.set_memory_limit(MAX_MEMORY)?;

        let budget = self.budget.clone();
        lua.set_hook(HookTriggers::new().every_nth_instruction(HOOK_EVERY), move |_lua, _debug| {
            let used = budget.get().saturating_add(HOOK_EVERY as u64);
            budget.set(used);
            if used > MAX_INSTRUCTIONS {
                Err(mlua::Error::runtime("too many operations (an infinite loop?)"))
            } else {
                Ok(VmState::Continue)
            }
        });

        let globals = lua.globals();
        // The base library is always loaded: what allows reading or loading code is removed.
        for name in ["dofile", "loadfile", "load", "loadstring", "require", "collectgarbage", "rawset", "rawget", "getmetatable", "setmetatable"] {
            globals.set(name, Value::Nil)?;
        }

        let output = self.output.clone();
        globals.set(
            "print",
            lua.create_function(move |_, args: Variadic<Value>| {
                let line = args.iter().map(|v| v.to_string().unwrap_or_else(|_| "?".into())).collect::<Vec<_>>().join("\t");
                let mut out = output.borrow_mut();
                if out.len() < MAX_OUTPUT_LINES {
                    out.push(line);
                }
                Ok(())
            })?,
        )?;

        // Orbes.
        let scene = self.scene.clone();
        globals.set(
            "orb",
            lua.create_function(move |_, (x, y, z): (f64, f64, f64)| {
                let mut list = scene.borrow_mut();
                if list.len() >= MAX_ORBS {
                    return Err(mlua::Error::runtime(format!("too many orbs (the maximum is {MAX_ORBS})")));
                }
                let handle = OrbHandle { id: list.len(), data: Rc::new(RefCell::new(ScriptOrb::new(x as f32, y as f32, z as f32))) };
                list.push(handle.clone());
                Ok(handle)
            })?,
        )?;
        let scene = self.scene.clone();
        globals.set("orbs", lua.create_function(move |_, ()| Ok(scene.borrow().clone()))?)?;

        // buffer: the open file, read only.
        let buffer = lua.create_table()?;
        let snap = self.snapshot.clone();
        buffer.set("text", lua.create_function(move |_, ()| Ok(snap.borrow().text.clone()))?)?;
        let snap = self.snapshot.clone();
        buffer.set("file", lua.create_function(move |_, ()| Ok(snap.borrow().file.clone()))?)?;
        let snap = self.snapshot.clone();
        buffer.set("lines", lua.create_function(move |_, ()| Ok(snap.borrow().text.lines().count()))?)?;
        let snap = self.snapshot.clone();
        buffer.set(
            "line",
            lua.create_function(move |_, n: i64| {
                let s = snap.borrow();
                Ok(usize::try_from(n).ok().filter(|&n| n >= 1).and_then(|n| s.text.lines().nth(n - 1)).map(str::to_string))
            })?,
        )?;
        globals.set("buffer", buffer)?;

        // editor: state and orders.
        let editor = lua.create_table()?;
        let snap = self.snapshot.clone();
        editor.set("mode", lua.create_function(move |_, ()| Ok(snap.borrow().mode.clone()))?)?;
        let snap = self.snapshot.clone();
        editor.set("cursor", lua.create_function(move |_, ()| Ok(snap.borrow().cursor))?)?;
        let commands = self.commands.clone();
        editor.set(
            "message",
            lua.create_function(move |_, text: String| {
                push_command(&commands, Command::Message(text));
                Ok(())
            })?,
        )?;
        let commands = self.commands.clone();
        editor.set(
            "command",
            lua.create_function(move |_, text: String| {
                push_command(&commands, Command::Ex(text.trim_start_matches(':').to_string()));
                Ok(())
            })?,
        )?;
        globals.set("editor", editor)?;

        // gesture: react to what you do in space.
        let gesture = lua.create_table()?;
        let handlers = self.handlers.clone();
        gesture.set(
            "on",
            lua.create_function(move |lua, (name, f): (String, Function)| {
                let mut map = handlers.borrow_mut();
                if map.values().map(Vec::len).sum::<usize>() >= MAX_HANDLERS {
                    return Err(mlua::Error::runtime(format!("too many gesture.on handlers (the maximum is {MAX_HANDLERS})")));
                }
                map.entry(name).or_default().push(lua.create_registry_value(f)?);
                Ok(())
            })?,
        )?;
        globals.set("gesture", gesture)?;

        // config: defaults that can be changed.
        globals.set("config", lua.create_userdata(ConfigHandle(self.config.clone()))?)?;
        drop(globals);
        Ok(lua)
    }

    /// Rebuilds the interpreter and runs `init` and then the script, if any. A failure in
    /// `init` does not prevent running the script. Returns the errors, if there were any.
    fn rebuild(&mut self) -> Vec<ScriptError> {
        self.stop();
        *self.config.borrow_mut() = Config::default();
        let lua = match self.sandbox() {
            Ok(lua) => lua,
            Err(e) => return vec![ScriptError::from_lua(&e, "script")],
        };
        let mut errors = Vec::new();
        for (chunk, source) in [("init", self.init.clone()), ("script", self.script.clone())] {
            let Some(source) = source else { continue };
            self.budget.set(0);
            if let Err(e) = lua.load(&source).set_name(format!("={chunk}")).exec() {
                errors.push(ScriptError::from_lua(&e, chunk));
            }
        }
        self.has_frame = lua.globals().get::<Option<Function>>("on_frame").ok().flatten().is_some();
        self.lua = Some(lua);
        errors
    }

    /// Sets `init.lua` (or removes it with `None`) and runs it.
    pub fn set_init(&mut self, source: Option<String>) -> Vec<ScriptError> {
        self.init = source;
        self.rebuild()
    }

    /// Runs `source` as the script (after `init`). It replaces any previous script.
    pub fn run_script(&mut self, source: &str) -> Vec<ScriptError> {
        self.script = Some(source.to_string());
        self.rebuild()
    }

    /// Runs `init` and the script again from scratch (hot reload).
    pub fn reload(&mut self) -> Vec<ScriptError> {
        self.rebuild()
    }

    /// Like [`Runtime::run_script`], but returning only the first error.
    pub fn load(&mut self, source: &str) -> Result<(), ScriptError> {
        self.run_script(source).into_iter().next().map_or(Ok(()), Err)
    }

    /// Editor state the script will see.
    pub fn set_snapshot(&mut self, snapshot: Snapshot) {
        *self.snapshot.borrow_mut() = snapshot;
    }

    /// Notifies an event to the functions registered with `gesture.on(name, ...)`.
    pub fn emit(&mut self, name: &str, args: &[EventArg]) -> Vec<ScriptError> {
        let Some(lua) = self.lua.as_ref() else { return Vec::new() };
        let map = self.handlers.borrow();
        let Some(keys) = map.get(name) else { return Vec::new() };
        let mut errors = Vec::new();
        for key in keys {
            self.budget.set(0);
            let call = || -> mlua::Result<()> {
                let f: Function = lua.registry_value(key)?;
                let values = args
                    .iter()
                    .map(|a| match a {
                        EventArg::Num(n) => Ok(Value::Number(*n)),
                        EventArg::Text(s) => lua.create_string(s).map(Value::String),
                    })
                    .collect::<mlua::Result<Vec<_>>>()?;
                f.call::<()>(MultiValue::from_vec(values))
            };
            if let Err(e) = call() {
                errors.push(ScriptError::from_lua(&e, "script"));
            }
        }
        errors
    }

    /// Calls the script's `on_frame(t, dt)`, if it defines it. After an error it stops calling it
    /// (the script stays shown as it was) and the error is returned only once.
    pub fn frame(&mut self, t: f32, dt: f32) -> Result<(), ScriptError> {
        let (true, Some(lua)) = (self.has_frame, self.lua.as_ref()) else { return Ok(()) };
        self.budget.set(0);
        let result = match lua.globals().get::<Option<Function>>("on_frame") {
            Ok(Some(f)) => f.call::<()>((t as f64, dt as f64)),
            Ok(None) => return Ok(()),
            Err(e) => Err(e),
        };
        result.map_err(|e| {
            self.has_frame = false;
            ScriptError::from_lua(&e, "script")
        })
    }

    /// Discards the interpreter and the scene (`init` and the script are kept, for `reload`).
    pub fn stop(&mut self) {
        self.lua = None;
        self.scene.borrow_mut().clear();
        self.handlers.borrow_mut().clear();
        self.commands.borrow_mut().clear();
        self.has_frame = false;
    }

    pub fn is_running(&self) -> bool {
        self.lua.is_some()
    }

    /// The orbs as they are now.
    pub fn orbs(&self) -> Vec<ScriptOrb> {
        self.scene.borrow().iter().map(|h| *h.data.borrow()).collect()
    }

    /// What the script has written with `print` since the last time.
    pub fn take_output(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.output.borrow_mut())
    }

    /// The orders the script has requested since the last time.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut *self.commands.borrow_mut())
    }

    /// The defaults as the script leaves them.
    /// Changes a `config` value from outside Lua (the `:font`, `:panel` and `:distance` commands).
    pub fn adjust_config(&mut self, key: &str, value: f32, relative: bool) -> Result<f32, String> {
        self.config.borrow_mut().adjust(key, value, relative)
    }

    pub fn config(&self) -> Config {
        *self.config.borrow()
    }
}

fn push_command(commands: &Rc<RefCell<Vec<Command>>>, command: Command) {
    let mut list = commands.borrow_mut();
    if list.len() < MAX_COMMANDS {
        list.push(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Runtime {
        let mut rt = Runtime::new();
        rt.load(source).unwrap_or_else(|e| panic!("{e}"));
        rt
    }

    #[test]
    fn a_script_creates_orbs_where_it_says() {
        let rt = run("local a = orb(0.1, -0.2, -0.5) local b = orb(1, 2, 3)");
        let orbs = rt.orbs();
        assert_eq!(orbs.len(), 2);
        assert_eq!(orbs[0].position, [0.1, -0.2, -0.5]);
        assert_eq!(orbs[1].position, [1.0, 2.0, 3.0], "integers work too");
    }

    #[test]
    fn properties_and_colors_are_set_and_clamped() {
        let rt = run("local a = orb(0,0,-1) a.radius = 9.0 a.intensity = 2 a:color(2.0, 0.5, -1.0) a:rim(0.1, 0.2, 0.3)");
        let o = rt.orbs()[0];
        assert_eq!(o.radius, 0.5, "the radius is clamped");
        assert_eq!(o.intensity, 2.0);
        assert_eq!(o.core, [1.0, 0.5, 0.0], "colors are clamped to 0..1");
        assert_eq!(o.rim, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn orb_fields_can_be_read_back() {
        let mut rt = run("local a = orb(0.5, 0, -1) print(a.x, a.id)");
        assert_eq!(rt.take_output(), vec!["0.5\t0".to_string()]);
    }

    #[test]
    fn on_frame_animates_the_scene() {
        let mut rt = run("orb(0,0,-1) function on_frame(t, dt) for _, o in ipairs(orbs()) do o.y = t * 2.0 end end");
        rt.frame(0.5, 0.014).unwrap();
        assert!((rt.orbs()[0].position[1] - 1.0).abs() < 1e-6);
        rt.frame(1.0, 0.014).unwrap();
        assert!((rt.orbs()[0].position[1] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn on_frame_keeps_state_in_upvalues() {
        let mut rt = run("local n = 0 local a = orb(0,0,-1) function on_frame(t, dt) n = n + 1 a.x = n end");
        for _ in 0..3 {
            rt.frame(0.0, 0.0).unwrap();
        }
        assert_eq!(rt.orbs()[0].position[0], 3.0);
    }

    #[test]
    fn a_script_without_on_frame_just_stays() {
        let mut rt = run("orb(0,0,-1)");
        assert!(rt.frame(1.0, 0.01).is_ok());
        assert_eq!(rt.orbs().len(), 1);
    }

    #[test]
    fn syntax_errors_report_the_line() {
        let err = Runtime::new().load("local a = 1\nlocal b = = 2\n").unwrap_err();
        assert_eq!(err.line, Some(2), "{err}");
    }

    #[test]
    fn runtime_errors_report_the_line() {
        let err = Runtime::new().load("local a = orb(0,0,-1)\nlocal b = no_existe(1)").unwrap_err();
        assert_eq!(err.line, Some(2), "{err}");
        assert!(!err.message.starts_with("script:"), "{}", err.message);
    }

    #[test]
    fn an_endless_loop_is_stopped() {
        let err = Runtime::new().load("while true do end").unwrap_err();
        assert!(err.message.contains("too many operations"), "{err}");
        let mut rt = run("function on_frame(t, dt) while true do end end");
        assert!(rt.frame(0.0, 0.0).is_err());
        assert!(rt.frame(0.0, 0.0).is_ok(), "after the error it is no longer called");
    }

    #[test]
    fn a_greedy_script_hits_the_memory_limit() {
        let err = Runtime::new().load("local t = {} for i = 1, 100000000 do t[i] = string.rep('x', 1000) end").unwrap_err();
        let msg = err.message.to_lowercase();
        assert!(msg.contains("memory") || msg.contains("operaciones"), "{err}");
    }

    #[test]
    fn there_is_a_cap_on_orbs() {
        let err = Runtime::new().load("for i = 1, 1000 do orb(0, 0, -1) end").unwrap_err();
        assert!(err.message.contains("too many orbs"), "{err}");
    }

    #[test]
    fn the_sandbox_has_no_files_system_or_code_loading() {
        for src in [
            "io.open('x')",
            "os.execute('x')",
            "require('x')",
            "load('return 1')()",
            "dofile('x')",
            "debug.traceback()",
            "package.loaded",
        ] {
            assert!(Runtime::new().load(src).is_err(), "{src} should not work");
        }
    }

    #[test]
    fn math_string_and_table_are_available() {
        let mut rt = run("print(math.sin(0), string.upper('a'), #table.pack(1, 2, 3))");
        assert_eq!(rt.take_output(), vec!["0\tA\t3".to_string()]);
    }

    #[test]
    fn print_goes_to_the_output() {
        let mut rt = run("print('hola') print(1 + 2)");
        assert_eq!(rt.take_output(), vec!["hola".to_string(), "3".to_string()]);
        assert!(rt.take_output().is_empty());
    }

    #[test]
    fn loading_again_replaces_the_previous_scene() {
        let mut rt = run("orb(0,0,-1) orb(0,0,-2)");
        rt.load("orb(5,5,5)").unwrap();
        assert_eq!(rt.orbs().len(), 1);
        rt.stop();
        assert!(rt.orbs().is_empty() && !rt.is_running());
    }

    #[test]
    fn a_failed_load_leaves_no_scene() {
        let mut rt = run("orb(0,0,-1)");
        assert!(rt.load("orb(1,1,1) local = ;").is_err());
        assert!(rt.orbs().is_empty());
    }

    // ----- buffer and editor -----

    fn snapshot() -> Snapshot {
        Snapshot { text: "uno\ndos\ntres\n".into(), file: "a.rs".into(), mode: "NORMAL".into(), cursor: (2, 3) }
    }

    #[test]
    fn a_script_can_read_the_open_buffer() {
        let mut rt = Runtime::new();
        rt.set_snapshot(snapshot());
        rt.load("print(buffer.file(), buffer.lines(), buffer.line(2), buffer.line(9), #buffer.text())").unwrap();
        assert_eq!(rt.take_output(), vec!["a.rs\t3\tdos\tnil\t13".to_string()]);
    }

    #[test]
    fn a_script_can_read_the_mode_and_cursor() {
        let mut rt = Runtime::new();
        rt.set_snapshot(snapshot());
        rt.load("local l, c = editor.cursor() print(editor.mode(), l, c)").unwrap();
        assert_eq!(rt.take_output(), vec!["NORMAL\t2\t3".to_string()]);
    }

    #[test]
    fn a_script_can_ask_for_messages_and_commands() {
        let mut rt = run("editor.message('hola') editor.command(':w') editor.command('e otro.rs')");
        assert_eq!(
            rt.take_commands(),
            vec![Command::Message("hola".into()), Command::Ex("w".into()), Command::Ex("e otro.rs".into())]
        );
        assert!(rt.take_commands().is_empty());
    }

    #[test]
    fn the_command_queue_is_bounded() {
        let mut rt = run("for i = 1, 1000 do editor.message('x') end");
        assert_eq!(rt.take_commands().len(), MAX_COMMANDS);
    }

    // ----- gestures and events -----

    #[test]
    fn gesture_handlers_receive_the_event_and_its_arguments() {
        let mut rt = run("gesture.on('card_tap', function(name, n) print(name, n) end)");
        let errors = rt.emit("card_tap", &[EventArg::Text("main.rs".into()), EventArg::Num(3.0)]);
        assert!(errors.is_empty());
        assert_eq!(rt.take_output(), vec!["main.rs\t3".to_string()]);
        assert!(rt.emit("other_event", &[]).is_empty(), "an event without handlers does nothing");
    }

    #[test]
    fn several_handlers_run_in_order_and_one_failing_does_not_stop_the_others() {
        let mut rt = run(
            "gesture.on('x', function() print('uno') end) gesture.on('x', function() error('fallo') end) gesture.on('x', function() print('tres') end)",
        );
        let errors = rt.emit("x", &[]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("fallo"), "{}", errors[0]);
        assert_eq!(rt.take_output(), vec!["uno".to_string(), "tres".to_string()]);
    }

    #[test]
    fn a_handler_can_use_the_whole_api() {
        let mut rt = Runtime::new();
        rt.set_snapshot(snapshot());
        rt.load("gesture.on('drawer_open', function() editor.message(buffer.file()) end)").unwrap();
        rt.emit("drawer_open", &[]);
        assert_eq!(rt.take_commands(), vec![Command::Message("a.rs".into())]);
    }

    #[test]
    fn an_endless_handler_is_stopped() {
        let mut rt = run("gesture.on('x', function() while true do end end)");
        let errors = rt.emit("x", &[]);
        assert!(errors[0].message.contains("too many operations"), "{}", errors[0]);
    }

    #[test]
    fn there_is_a_cap_on_handlers() {
        let err = Runtime::new().load("for i = 1, 1000 do gesture.on('x', function() end) end").unwrap_err();
        assert!(err.message.contains("too many gesture.on"), "{err}");
    }

    // ----- configuration -----

    #[test]
    fn config_has_the_defaults_until_the_user_changes_them() {
        let rt = run("");
        assert_eq!(rt.config(), Config::default());
        assert_eq!(rt.config().card_release, CardRelease::Return);
        assert_eq!(rt.config().keyboard, KeyboardKind::Own);
    }

    #[test]
    fn the_reading_settings_have_defaults_ranges_and_can_be_adjusted_from_outside() {
        let rt = run("");
        let c = rt.config();
        assert_eq!((c.panel_width, c.panel_distance, c.font_scale), (1.2, 0.9, 1.0));
        let rt = run("config.panel_width = 9 config.panel_distance = 0 config.font_scale = 1.25");
        let c = rt.config();
        assert_eq!((c.panel_width, c.panel_distance, c.font_scale), (2.4, 0.5, 1.25), "they are clamped to their range");

        let mut rt = run("print(config.font_scale, config.panel_width)");
        assert_eq!(rt.take_output(), vec!["1\t1.2".to_string()]);
        assert_eq!(rt.adjust_config("font_scale", 0.1, true), Ok(1.1));
        assert_eq!(rt.adjust_config("font_scale", 5.0, true), Ok(2.0), "relative and clamped");
        assert_eq!(rt.adjust_config("panel_width", 1.6, false), Ok(1.6));
        assert_eq!(rt.config().panel_width, 1.6);
        assert!(rt.adjust_config("gravity", 1.0, false).is_err(), "only these three change this way");
        assert!(rt.adjust_config("font_scale", f32::NAN, false).is_err());
    }

    #[test]
    fn the_keyboard_can_be_switched_and_rejects_unknown_ones() {
        let rt = run("config.keyboard = 'system'");
        assert_eq!(rt.config().keyboard, KeyboardKind::System);
        let mut rt = run("config.keyboard = 'system' config.keyboard = 'own' print(config.keyboard)");
        assert_eq!(rt.config().keyboard, KeyboardKind::Own);
        assert_eq!(rt.take_output(), vec!["own".to_string()]);
        let err = Runtime::new().load("config.keyboard = 'holo'").unwrap_err();
        assert!(err.message.contains("'own' or 'system'"), "{err}");
    }

    #[test]
    fn config_values_are_changed_and_clamped() {
        let rt = run("config.fan_radius = 0.6 config.gravity = 100 config.card_width = 0.01 config.card_release = 'stay' config.throw_min_speed = 1.5");
        let c = rt.config();
        assert_eq!(c.fan_radius, 0.6);
        assert_eq!(c.gravity, 20.0, "it is clamped");
        assert_eq!(c.card_width, 0.10, "it is clamped");
        assert_eq!(c.card_release, CardRelease::Stay);
        assert_eq!(c.throw_min_speed, 1.5);
    }

    #[test]
    fn config_can_be_read_back_and_rejects_bad_values() {
        let mut rt = run("print(config.fan_radius, config.card_release)");
        assert_eq!(rt.take_output(), vec!["0.62\treturn".to_string()]);
        let err = Runtime::new().load("config.card_release = 'volar'").unwrap_err();
        assert!(err.message.contains("'return' or 'stay'"), "{err}");
        let err = Runtime::new().load("config.no_existe = 1").unwrap_err();
        assert_eq!(err.line, Some(1), "{err}");
    }

    // ----- init.lua, recarga -----

    #[test]
    fn init_runs_first_and_the_script_after() {
        let mut rt = Runtime::new();
        assert!(rt.set_init(Some("print('init') config.fan_radius = 0.7".into())).is_empty());
        assert!(rt.run_script("print('script') print(config.fan_radius)").is_empty());
        assert_eq!(rt.take_output(), vec!["init".to_string(), "init".to_string(), "script".to_string(), "0.7".to_string()]);
    }

    #[test]
    fn a_broken_init_does_not_stop_the_script_and_names_itself() {
        let mut rt = Runtime::new();
        rt.set_init(Some("local x = = 1".into()));
        let errors = rt.run_script("orb(0,0,-1)");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].chunk, "init");
        assert!(errors[0].to_string().starts_with("init.lua, line 1"), "{}", errors[0]);
        assert_eq!(rt.orbs().len(), 1, "the script still ran");
    }

    #[test]
    fn reload_rebuilds_everything_and_forgets_removed_settings() {
        let mut rt = Runtime::new();
        rt.set_init(Some("config.gravity = 9".into()));
        assert_eq!(rt.config().gravity, 9.0);
        rt.set_init(Some("-- no changes".into()));
        assert_eq!(rt.config().gravity, Config::default().gravity, "removing the line restores the default");

        rt.run_script("gesture.on('x', function() print('a') end) orb(0,0,-1)");
        assert_eq!(rt.orbs().len(), 1);
        assert!(rt.reload().is_empty());
        rt.emit("x", &[]);
        assert_eq!(rt.take_output(), vec!["a".to_string()], "after reloading there is a single handler, not two");
    }

    #[test]
    fn a_script_error_names_the_script_not_the_init() {
        let mut rt = Runtime::new();
        rt.set_init(Some("print('bien')".into()));
        let errors = rt.run_script("local a = 1\nerror('boom')");
        assert_eq!(errors[0].chunk, "script");
        assert_eq!(errors[0].line, Some(2));
    }
}
