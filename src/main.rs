use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, fs, io::Cursor, path::PathBuf};
use tiny_http::{Header, Method, Response};

const DEVICE_ID: &str = "vizio-ctl-rs";
const PORTS: [u16; 2] = [9000, 7345];

#[derive(Parser)]
#[command(
    name = "vizio-ctl",
    version,
    about = "Control a Vizio SmartCast soundbar over the LAN",
    long_about = "Control a Vizio SmartCast soundbar over the LAN.\n\n\
        Soundbars accept commands without pairing, so --host is all you need.\n\
        Run `vizio-ctl <command> --help` for details on a command.",
    after_help = "Examples:\n  \
        vizio-ctl --host 192.168.1.50 status\n  \
        vizio-ctl --host 192.168.1.50 vol-up 3\n  \
        vizio-ctl --host 192.168.1.50 mute"
)]
struct Cli {
    /// Soundbar IP address (e.g. 192.168.1.50); overrides any saved config
    #[arg(long, global = true, value_name = "IP")]
    host: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Pair with the soundbar and save an auth token (needs a PIN; usually unnecessary)
    Pair,
    /// Show whether the soundbar is powered on or off
    Status,
    /// Turn the soundbar on
    On,
    /// Turn the soundbar off (standby)
    Off,
    /// Raise the volume
    VolUp {
        /// Number of volume steps to raise
        #[arg(default_value_t = 1, value_name = "STEPS")]
        steps: u32,
    },
    /// Lower the volume
    VolDown {
        /// Number of volume steps to lower
        #[arg(default_value_t = 1, value_name = "STEPS")]
        steps: u32,
    },
    /// Toggle mute on or off
    Mute,
    /// Show or set bass (-12 to 12)
    Bass {
        /// New level from -12 to 12; omit to show the current level
        #[arg(allow_hyphen_values = true, value_name = "LEVEL")]
        value: Option<i64>,
    },
    /// Show or set treble (-12 to 12)
    Treble {
        /// New level from -12 to 12; omit to show the current level
        #[arg(allow_hyphen_values = true, value_name = "LEVEL")]
        value: Option<i64>,
    },
    /// Show or set surround balance (-12 to 12)
    Balance {
        /// New level from -12 to 12; omit to show the current level
        #[arg(allow_hyphen_values = true, value_name = "LEVEL")]
        value: Option<i64>,
    },
    /// Show or set any setting by name
    #[command(after_help = "Settings:\n  \
        bass, treble, balance, center, surround-volume, subwoofer   -12 to 12\n  \
        av-delay                                                    0 to 8\n  \
        eq                                                          Movie, Music or Direct\n  \
        vrt-x, surround-sound, night-mode                           On or Off\n  \
        input                                                       e.g. HDMI-ARC, Optical, Bluetooth\n  \
        sleep-timer, eco-power, power-led, tv-remote                choices come from the device\n  \
        language, country, timezone, dst                            choices come from the device\n\n\
        Examples:\n  vizio-ctl --host 192.168.1.50 set eq Music\n  \
        vizio-ctl --host 192.168.1.50 set subwoofer -2\n  \
        vizio-ctl --host 192.168.1.50 set input Optical")]
    Set {
        /// Setting name (see list below)
        name: String,
        /// New value; omit to show the current value
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,
    },
    /// Run an action such as a speaker test
    #[command(after_help = "Actions:\n  \
        speaker-test-auto, speaker-test-left, speaker-test-center, speaker-test-right,\n  \
        speaker-test-left-surround, speaker-test-right-surround, speaker-test-subwoofer,\n  \
        input-auto-detect, bluetooth-pair, subwoofer-pair, check-update, power-cycle")]
    Action {
        /// Action name (see list below)
        name: String,
    },
    /// Show model, firmware, Cast and network information
    Info {
        /// Show serial number, MAC addresses, SSID and IP (hidden by default)
        #[arg(long)]
        reveal: bool,
    },
    /// Start a local web UI for controlling the soundbar
    Serve {
        /// Address to listen on (use 0.0.0.0 to allow other devices on your LAN)
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// Port to listen on
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
}

#[derive(Serialize, Deserialize)]
struct Config {
    host: String,
    port: u16,
    auth_token: String,
}

fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("no config dir")?.join("vizio-ctl");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("config.json"))
}

fn save_config(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    fs::write(&path, serde_json::to_string_pretty(cfg)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_config() -> Result<Config> {
    let raw = fs::read_to_string(config_path()?).context("not paired yet; run `pair --host <ip>`")?;
    Ok(serde_json::from_str(&raw)?)
}

// The soundbar uses a self-signed certificate.
fn client() -> Result<Client> {
    Ok(Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(10))
        .build()?)
}

fn find_port(c: &Client, host: &str) -> Result<u16> {
    let mut last_err = None;
    for port in PORTS {
        let url = format!("https://{host}:{port}/state/device/deviceinfo");
        match c.get(&url).send() {
            Ok(_) => return Ok(port),
            Err(e) => last_err = Some(format!("port {port}: {e:?}")),
        }
    }
    bail!(
        "no SmartCast API on {host} (tried ports 9000, 7345); last error: {}",
        last_err.unwrap_or_default()
    )
}

fn pair(c: &Client, host: &str) -> Result<()> {
    let port = find_port(c, host)?;
    let base = format!("https://{host}:{port}");

    let start: Value = c
        .put(format!("{base}/pairing/start"))
        .json(&json!({"DEVICE_NAME": "vizio-ctl", "DEVICE_ID": DEVICE_ID}))
        .send()?
        .json()?;
    let req_token = start["ITEM"]["PAIRING_REQ_TOKEN"]
        .as_u64()
        .context("pairing/start returned no token")?;

    println!("Enter the PIN shown on the soundbar:");
    let mut pin = String::new();
    std::io::stdin().read_line(&mut pin)?;

    let done: Value = c
        .put(format!("{base}/pairing/pair"))
        .json(&json!({
            "DEVICE_ID": DEVICE_ID,
            "CHALLENGE_TYPE": 1,
            "RESPONSE_VALUE": pin.trim(),
            "PAIRING_REQ_TOKEN": req_token,
        }))
        .send()?
        .json()?;
    let token = done["ITEM"]["AUTH_TOKEN"]
        .as_str()
        .context("pairing failed (wrong PIN?)")?;

    save_config(&Config { host: host.into(), port, auth_token: token.into() })?;
    println!("Paired. Token saved to {}", config_path()?.display());
    Ok(())
}

fn key(c: &Client, cfg: &Config, codeset: u32, code: u32) -> Result<()> {
    let resp: Value = c
        .put(format!("https://{}:{}/key_command/", cfg.host, cfg.port))
        .header("AUTH", &cfg.auth_token)
        .json(&json!({"KEYLIST": [{"CODESET": codeset, "CODE": code, "ACTION": "KEYPRESS"}]}))
        .send()?
        .json()?;
    if resp["STATUS"]["RESULT"] != "SUCCESS" {
        bail!("device rejected command: {}", resp["STATUS"]);
    }
    Ok(())
}

fn power_on(c: &Client, cfg: &Config) -> Result<bool> {
    let v: Value = c
        .get(format!("https://{}:{}/state/device/power_mode", cfg.host, cfg.port))
        .header("AUTH", &cfg.auth_token)
        .send()?
        .json()?;
    Ok(v["ITEMS"][0]["VALUE"] == 1)
}

// Codesets: 5 = audio (vol/mute), 11 = power
fn volume(c: &Client, cfg: &Config, up: bool, steps: u32) -> Result<()> {
    (0..steps).try_for_each(|_| key(c, cfg, 5, if up { 1 } else { 0 }))
}

const ROOT: &str = "menu_native/dynamic/audio_settings";
const STATIC_ROOT: &str = "menu_native/static/audio_settings";

enum Kind {
    Range(i64, i64),
    Choice(&'static [&'static str]),
    /// Choices are read from the soundbar's static menu.
    Dynamic,
    /// Choices are the input names listed under the input menu.
    Input,
    Action { confirm: bool },
}

struct Setting {
    name: &'static str,  // CLI/API name
    path: &'static str,  // menu path under audio_settings; last segment is the soundbar's own name
    label: &'static str,
    group: &'static str,
    kind: Kind,
}

const SETTINGS: &[Setting] = &[
    Setting { name: "bass", path: "audio/bass", label: "Bass", group: "Sound", kind: Kind::Range(-12, 12) },
    Setting { name: "treble", path: "audio/treble", label: "Treble", group: "Sound", kind: Kind::Range(-12, 12) },
    Setting { name: "balance", path: "audio/surround_balance", label: "Surround balance", group: "Sound", kind: Kind::Range(-12, 12) },
    Setting { name: "center", path: "audio/center", label: "Center", group: "Sound", kind: Kind::Range(-12, 12) },
    Setting { name: "surround-volume", path: "audio/surround_volume", label: "Surround volume", group: "Sound", kind: Kind::Range(-12, 12) },
    Setting { name: "subwoofer", path: "audio/subwoofer", label: "Subwoofer", group: "Sound", kind: Kind::Range(-12, 12) },
    Setting { name: "av-delay", path: "audio/av_delay", label: "AV delay", group: "Sound", kind: Kind::Range(0, 8) },
    Setting { name: "eq", path: "audio/eq", label: "EQ", group: "Sound", kind: Kind::Choice(&["Movie", "Music", "Direct"]) },
    Setting { name: "vrt-x", path: "audio/virtualx", label: "VRT-X", group: "Sound", kind: Kind::Choice(&["Off", "On"]) },
    Setting { name: "surround-sound", path: "audio/surround_sound", label: "Surround sound", group: "Sound", kind: Kind::Choice(&["Off", "On"]) },
    Setting { name: "night-mode", path: "audio/night_mode", label: "Night mode", group: "Sound", kind: Kind::Choice(&["Off", "On"]) },
    Setting { name: "input", path: "input/current_input", label: "Input", group: "Input", kind: Kind::Input },
    Setting { name: "sleep-timer", path: "system/sleep_timer", label: "Sleep timer", group: "System", kind: Kind::Dynamic },
    Setting { name: "eco-power", path: "system/eco_power", label: "Eco power", group: "System", kind: Kind::Dynamic },
    Setting { name: "power-led", path: "system/power_indicator", label: "Power indicator LED", group: "System", kind: Kind::Dynamic },
    Setting { name: "tv-remote", path: "system/vizio_tv_remote", label: "Vizio TV remote", group: "System", kind: Kind::Dynamic },
    Setting { name: "language", path: "system/menu_language", label: "Menu language", group: "System", kind: Kind::Dynamic },
    Setting { name: "country", path: "system/country", label: "Country", group: "System", kind: Kind::Dynamic },
    Setting { name: "timezone", path: "system/local_time_settings/timezone", label: "Time zone", group: "System", kind: Kind::Dynamic },
    Setting { name: "dst", path: "system/local_time_settings/daylight_savings_time", label: "Daylight savings", group: "System", kind: Kind::Dynamic },
    Setting { name: "speaker-test-auto", path: "system/speaker_test/automatic", label: "Speaker test: all", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "speaker-test-left", path: "system/speaker_test/left", label: "Test left", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "speaker-test-center", path: "system/speaker_test/center", label: "Test center", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "speaker-test-right", path: "system/speaker_test/right", label: "Test right", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "speaker-test-left-surround", path: "system/speaker_test/left_surround", label: "Test left surround", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "speaker-test-right-surround", path: "system/speaker_test/right_surround", label: "Test right surround", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "speaker-test-subwoofer", path: "system/speaker_test/subwoofer", label: "Test subwoofer", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "input-auto-detect", path: "system/input_auto_detect", label: "Input auto detect", group: "Actions", kind: Kind::Action { confirm: false } },
    Setting { name: "bluetooth-pair", path: "audio/bluetooth_pair", label: "Bluetooth pairing", group: "Actions", kind: Kind::Action { confirm: true } },
    Setting { name: "subwoofer-pair", path: "system/subwoofer_pair", label: "Subwoofer pairing", group: "Actions", kind: Kind::Action { confirm: true } },
    Setting { name: "check-update", path: "system/check_for_update", label: "Check for updates", group: "Actions", kind: Kind::Action { confirm: true } },
    Setting { name: "power-cycle", path: "system/service_reset", label: "Soft power cycle", group: "Actions", kind: Kind::Action { confirm: true } },
];

fn is_action(s: &Setting) -> bool {
    matches!(s.kind, Kind::Action { .. })
}

fn setting_by_name(name: &str) -> Result<&'static Setting> {
    SETTINGS.iter().find(|s| s.name == name).with_context(|| {
        let names: Vec<_> = SETTINGS.iter().map(|s| s.name).collect();
        format!("unknown name '{name}'; valid: {}", names.join(", "))
    })
}

fn split_path(path: &'static str) -> (&'static str, &'static str) {
    path.rsplit_once('/').expect("setting paths contain a menu")
}

fn get_json(c: &Client, cfg: &Config, root: &str, path: &str) -> Result<Value> {
    Ok(c.get(format!("https://{}:{}/{root}/{path}", cfg.host, cfg.port))
        .header("AUTH", &cfg.auth_token)
        .send()?
        .json()?)
}

fn menu_items(c: &Client, cfg: &Config, menu: &str) -> Result<Vec<Value>> {
    let v = get_json(c, cfg, ROOT, menu)?;
    if v["STATUS"]["RESULT"] != "SUCCESS" {
        bail!("could not read menu {menu}: {}", v["STATUS"]);
    }
    Ok(v["ITEMS"].as_array().cloned().unwrap_or_default())
}

fn find_item<'a>(items: &'a [Value], cname: &str) -> Option<&'a Value> {
    items.iter().find(|i| i["CNAME"] == cname)
}

fn options_for(c: &Client, cfg: &Config, s: &Setting) -> Result<Vec<String>> {
    match &s.kind {
        Kind::Choice(opts) => Ok(opts.iter().map(|o| o.to_string()).collect()),
        Kind::Dynamic => {
            let v = get_json(c, cfg, STATIC_ROOT, s.path)?;
            let elements = v["ITEMS"][0]["ELEMENTS"].as_array().context("device reports no choices")?;
            Ok(elements.iter().filter_map(|e| e.as_str().map(String::from)).collect())
        }
        Kind::Input => {
            let items = menu_items(c, cfg, "input")?;
            Ok(items
                .iter()
                .filter(|i| i["TYPE"] == "T_DEVICE_V1")
                .filter_map(|i| i["NAME"].as_str().map(String::from))
                .collect())
        }
        _ => bail!("{} has no choices", s.name),
    }
}

fn build_schema(c: &Client, cfg: &Config) -> Value {
    let mut out = Vec::new();
    for s in SETTINGS {
        let entry = match &s.kind {
            Kind::Range(min, max) => Some(json!({
                "name": s.name, "label": s.label, "group": s.group, "kind": "range", "min": min, "max": max
            })),
            Kind::Action { confirm } => Some(json!({
                "name": s.name, "label": s.label, "group": s.group, "kind": "action", "confirm": confirm
            })),
            _ => options_for(c, cfg, s).ok().filter(|o| !o.is_empty()).map(|o| {
                json!({"name": s.name, "label": s.label, "group": s.group, "kind": "choice", "options": o})
            }),
        };
        if let Some(e) = entry {
            out.push(e);
        }
    }
    json!({"ok": true, "settings": out})
}

fn set_setting(c: &Client, cfg: &Config, name: &str, raw: &str) -> Result<()> {
    let setting = setting_by_name(name)?;
    let value = match &setting.kind {
        Kind::Range(min, max) => {
            let v: i64 = raw.parse().with_context(|| format!("{name} needs a whole number"))?;
            if !(*min..=*max).contains(&v) {
                bail!("{name} must be between {min} and {max}");
            }
            json!(v)
        }
        Kind::Action { .. } => Value::Null,
        _ => {
            let opts = options_for(c, cfg, setting)?;
            let choice = opts
                .iter()
                .find(|o| o.eq_ignore_ascii_case(raw))
                .with_context(|| format!("{name} must be one of: {}", opts.join(", ")))?;
            json!(choice)
        }
    };

    let (menu, cname) = split_path(setting.path);
    let items = menu_items(c, cfg, menu)?;
    let hash = find_item(&items, cname)
        .and_then(|i| i["HASHVAL"].as_u64())
        .with_context(|| format!("{name} not found on device"))?;
    let body = if is_action(setting) {
        json!({"REQUEST": "ACTION", "HASHVAL": hash})
    } else {
        json!({"REQUEST": "MODIFY", "VALUE": value, "HASHVAL": hash})
    };
    let resp: Value = c
        .put(format!("https://{}:{}/{ROOT}/{}", cfg.host, cfg.port, setting.path))
        .header("AUTH", &cfg.auth_token)
        .json(&body)
        .send()?
        .json()?;
    if resp["STATUS"]["RESULT"] != "SUCCESS" {
        bail!("device rejected change: {}", resp["STATUS"]);
    }
    Ok(())
}

fn run_action(c: &Client, cfg: &Config, name: &str) -> Result<()> {
    if !is_action(setting_by_name(name)?) {
        bail!("{name} is a setting, not an action; use `set {name} <value>`");
    }
    set_setting(c, cfg, name, "")
}

fn state(c: &Client, cfg: &Config) -> Result<Value> {
    let on = power_on(c, cfg)?;
    // Menus may be unreadable while the soundbar is off; those values are reported as missing.
    let mut menus: HashMap<&'static str, Vec<Value>> = HashMap::new();
    let mut settings = serde_json::Map::new();
    for s in SETTINGS.iter().filter(|s| !is_action(s)) {
        let (menu, cname) = split_path(s.path);
        let items = menus.entry(menu).or_insert_with(|| menu_items(c, cfg, menu).unwrap_or_default());
        if let Some(item) = find_item(items, cname) {
            settings.insert(s.name.into(), item["VALUE"].clone());
        }
    }
    let audio = menus.entry("audio").or_insert_with(|| menu_items(c, cfg, "audio").unwrap_or_default());
    Ok(json!({
        "power": if on { "on" } else { "off" },
        "volume": find_item(audio, "volume").and_then(|i| i["VALUE"].as_i64()),
        "mute": find_item(audio, "mute").map(|i| i["VALUE"] == "On"),
        "settings": settings,
    }))
}

const SENSITIVE: [&str; 9] = ["serial", "mac", "ssid", "ip_address", "esn", "uuid", "token", "password", "device_id"];

fn info(c: &Client, cfg: &Config, reveal: bool) -> Result<Value> {
    let sections = [
        ("Soundbar", "system/system_information/speaker_information"),
        ("Network", "system/system_information/network_information"),
        ("Diagnostics", "system/system_information/uli_information"),
        ("Cast", "cast"),
    ];
    let mut out = Vec::new();
    for (title, menu) in sections {
        let Ok(items) = menu_items(c, cfg, menu) else { continue };
        let rows: Vec<Value> = items
            .iter()
            .filter(|i| i["TYPE"] != "T_MENU_V1" && i["TYPE"] != "T_ACTION_V1")
            .map(|i| {
                let cname = i["CNAME"].as_str().unwrap_or("").to_lowercase();
                let label = i["NAME"].as_str().unwrap_or("").trim_end_matches(':').trim().to_string();
                let value = match &i["VALUE"] {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                let hidden = !reveal && SENSITIVE.iter().any(|k| cname.contains(k));
                json!({"label": label, "value": if hidden { "hidden".to_string() } else { value }, "hidden": hidden})
            })
            .collect();
        out.push(json!({"title": title, "rows": rows}));
    }
    Ok(json!({"ok": true, "sections": out}))
}

fn setting_cmd(c: &Client, cfg: &Config, name: &str, value: Option<&str>) -> Result<()> {
    let setting = setting_by_name(name)?;
    if is_action(setting) {
        bail!("{name} is an action; use `action {name}`");
    }
    if let Some(v) = value {
        set_setting(c, cfg, name, v)?;
    }
    let (menu, cname) = split_path(setting.path);
    let items = menu_items(c, cfg, menu)?;
    match find_item(&items, cname) {
        Some(item) => println!("{}: {}", setting.name, item["VALUE"].to_string().trim_matches('"')),
        None => bail!("could not read {name}"),
    }
    Ok(())
}

fn print_status(c: &Client, cfg: &Config) -> Result<()> {
    let s = state(c, cfg)?;
    println!("power: {}", s["power"].as_str().unwrap_or("?"));
    if let Some(v) = s["volume"].as_i64() {
        println!("volume: {v}");
    }
    if let Some(m) = s["mute"].as_bool() {
        println!("mute: {}", if m { "on" } else { "off" });
    }
    for setting in SETTINGS.iter().filter(|s| !is_action(s)) {
        if let Some(v) = s["settings"].get(setting.name) {
            println!("{}: {}", setting.name, v.to_string().trim_matches('"'));
        }
    }
    Ok(())
}

fn print_info(c: &Client, cfg: &Config, reveal: bool) -> Result<()> {
    let v = info(c, cfg, reveal)?;
    for section in v["sections"].as_array().into_iter().flatten() {
        println!("{}", section["title"].as_str().unwrap_or(""));
        for row in section["rows"].as_array().into_iter().flatten() {
            println!("  {}: {}", row["label"].as_str().unwrap_or(""), row["value"].as_str().unwrap_or(""));
        }
    }
    Ok(())
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
}

fn json_response(status: u16, body: Value) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
}

fn handle_api(
    c: &Client,
    cfg: &Config,
    schema: &mut Option<Value>,
    method: &Method,
    path: &str,
    query: &str,
) -> Response<Cursor<Vec<u8>>> {
    let steps = query_param(query, "steps")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 20);

    let result = match (method, path) {
        (Method::Get, "/api/schema") => {
            let s = schema.get_or_insert_with(|| build_schema(c, cfg));
            return json_response(200, s.clone());
        }
        (Method::Get, "/api/info") => {
            let reveal = query_param(query, "reveal") == Some("1");
            return match info(c, cfg, reveal) {
                Ok(v) => json_response(200, v),
                Err(e) => json_response(502, json!({"ok": false, "error": e.to_string()})),
            };
        }
        (Method::Get, "/api/status") => Ok(()),
        (Method::Post, "/api/on") => key(c, cfg, 11, 2),
        (Method::Post, "/api/off") => key(c, cfg, 11, 0),
        (Method::Post, "/api/vol-up") => volume(c, cfg, true, steps),
        (Method::Post, "/api/vol-down") => volume(c, cfg, false, steps),
        (Method::Post, "/api/mute") => key(c, cfg, 5, 2),
        (Method::Post, p) if p.starts_with("/api/set/") => {
            let name = &p["/api/set/".len()..];
            match setting_by_name(name) {
                Ok(s) if !is_action(s) => {}
                _ => return json_response(404, json!({"ok": false, "error": "unknown setting"})),
            }
            match query_param(query, "value") {
                Some(v) => set_setting(c, cfg, name, &url_decode(v)),
                None => return json_response(400, json!({"ok": false, "error": "missing value"})),
            }
        }
        (Method::Post, p) if p.starts_with("/api/action/") => {
            let name = &p["/api/action/".len()..];
            match setting_by_name(name) {
                Ok(s) if is_action(s) => run_action(c, cfg, name),
                _ => return json_response(404, json!({"ok": false, "error": "unknown action"})),
            }
        }
        _ => return json_response(404, json!({"ok": false, "error": "not found"})),
    };

    match result.and_then(|_| state(c, cfg)) {
        Ok(mut s) => {
            s["ok"] = json!(true);
            json_response(200, s)
        }
        Err(e) => json_response(502, json!({"ok": false, "error": e.to_string()})),
    }
}

fn serve(c: &Client, cfg: &Config, bind: &str, port: u16) -> Result<()> {
    let addr = format!("{bind}:{port}");
    let server = tiny_http::Server::http(&addr).map_err(|e| anyhow!("cannot listen on {addr}: {e}"))?;
    println!("Serving http://{addr} (soundbar {}). Ctrl+C to stop.", cfg.host);

    // Reading every setting's choices takes a few requests, so build the list once.
    let mut schema: Option<Value> = None;

    for req in server.incoming_requests() {
        let method = req.method().clone();
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));

        let resp = if method == Method::Get && path == "/" {
            Response::from_string(include_str!("index.html"))
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap())
        } else if path.starts_with("/api/") {
            // Browsers can't send a custom header cross-origin without a preflight,
            // which this server never approves, so this blocks other sites from
            // driving the soundbar through the user's browser.
            let marked = req
                .headers()
                .iter()
                .any(|h| h.field.as_str().as_str().eq_ignore_ascii_case("x-requested-with"));
            if marked {
                handle_api(c, cfg, &mut schema, &method, path, query)
            } else {
                json_response(403, json!({"ok": false, "error": "missing X-Requested-With header"}))
            }
        } else {
            json_response(404, json!({"ok": false, "error": "not found"}))
        };
        let _ = req.respond(resp);
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let c = client()?;

    if let Cmd::Pair = cli.cmd {
        let host = cli.host.context("--host is required for pair")?;
        return pair(&c, &host);
    }

    // Soundbars/speakers accept commands without a token, so fall back to an
    // unauthenticated config when not paired.
    let mut cfg = match load_config() {
        Ok(cfg) => cfg,
        Err(_) => {
            let host = cli.host.clone().context("--host is required (not paired)")?;
            let port = find_port(&c, &host)?;
            Config { host, port, auth_token: String::new() }
        }
    };
    if let Some(h) = cli.host {
        cfg.host = h;
    }

    match cli.cmd {
        Cmd::Pair => unreachable!(),
        Cmd::Status => print_status(&c, &cfg)?,
        Cmd::On => key(&c, &cfg, 11, 2)?,
        Cmd::Off => key(&c, &cfg, 11, 0)?,
        Cmd::VolUp { steps } => volume(&c, &cfg, true, steps)?,
        Cmd::VolDown { steps } => volume(&c, &cfg, false, steps)?,
        Cmd::Mute => key(&c, &cfg, 5, 2)?,
        Cmd::Bass { value } => setting_cmd(&c, &cfg, "bass", value.map(|v| v.to_string()).as_deref())?,
        Cmd::Treble { value } => setting_cmd(&c, &cfg, "treble", value.map(|v| v.to_string()).as_deref())?,
        Cmd::Balance { value } => setting_cmd(&c, &cfg, "balance", value.map(|v| v.to_string()).as_deref())?,
        Cmd::Set { name, value } => setting_cmd(&c, &cfg, &name, value.as_deref())?,
        Cmd::Action { name } => run_action(&c, &cfg, &name)?,
        Cmd::Info { reveal } => print_info(&c, &cfg, reveal)?,
        Cmd::Serve { bind, port } => serve(&c, &cfg, &bind, port)?,
    }
    Ok(())
}
