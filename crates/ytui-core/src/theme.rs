use anyhow::{Context, Result, ensure};
use serde_json::Value;
pub type Rgb = (u8, u8, u8);
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Palette {
    pub name: String,
    pub source: String,
    pub background: Rgb,
    pub surface: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub selection: Rgb,
    pub error: Rgb,
    pub warning: Rgb,
    pub success: Rgb,
    pub light: bool,
}
pub const THEMES: &[&str] = &[
    "system",
    "catppuccin-mocha",
    "catppuccin-latte",
    "tokyo-night",
    "gruvbox",
    "gruvbox-light",
    "nord",
    "rose-pine",
    "ytui-dark",
    "ytui-light",
    "dracula",
    "solarized-dark",
    "terminal",
];
pub fn color(value: &str) -> Result<Rgb> {
    let value = value.trim().trim_start_matches('#');
    let value = if value.len() == 3 {
        value.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        value.into()
    };
    ensure!(
        value.len() == 6 && value.is_ascii(),
        "Invalid palette color"
    );
    Ok((
        u8::from_str_radix(&value[0..2], 16)?,
        u8::from_str_radix(&value[2..4], 16)?,
        u8::from_str_radix(&value[4..6], 16)?,
    ))
}
pub fn builtin(name: &str) -> Result<Palette> {
    let name = match name {
        "default" | "midnight" | "dark" => "ytui-dark",
        "light" => "ytui-light",
        other => other,
    };
    let (values, light) = match name {
        "catppuccin-mocha" | "system" => (
            [
                "1e1e2e", "313244", "cdd6f4", "a6adc8", "cba6f7", "45475a", "f38ba8", "f9e2af",
                "a6e3a1",
            ],
            false,
        ),
        "catppuccin-latte" => (
            [
                "eff1f5", "e6e9ef", "4c4f69", "66697f", "8839ef", "ccd0da", "d20f39", "946000",
                "2d7a1f",
            ],
            true,
        ),
        "tokyo-night" => (
            [
                "1a1b26", "24283b", "c0caf5", "9aa5ce", "7aa2f7", "3b4261", "f7768e", "e0af68",
                "9ece6a",
            ],
            false,
        ),
        "gruvbox" => (
            [
                "282828", "3c3836", "ebdbb2", "bdae93", "fabd2f", "504945", "fb4934", "fe8019",
                "b8bb26",
            ],
            false,
        ),
        "gruvbox-light" => (
            [
                "fbf1c7", "ebdbb2", "3c3836", "665c54", "9d0006", "d5c4a1", "cc241d", "af3a03",
                "427b58",
            ],
            true,
        ),
        "nord" => (
            [
                "2e3440", "3b4252", "eceff4", "c2c9d6", "88c0d0", "4c566a", "bf616a", "ebcb8b",
                "a3be8c",
            ],
            false,
        ),
        "rose-pine" => (
            [
                "191724", "1f1d2e", "e0def4", "aaa5c5", "c4a7e7", "403d52", "eb6f92", "f6c177",
                "9ccfd8",
            ],
            false,
        ),
        "ytui-dark" => (
            [
                "101418", "1b232c", "e5edf4", "a8b9c9", "6ddbc1", "294053", "ff7f86", "f5c36c",
                "8edd99",
            ],
            false,
        ),
        "ytui-light" => (
            [
                "f6f8fa", "e9eff3", "202c36", "52687b", "006f62", "d0e3eb", "b42336", "986300",
                "24713e",
            ],
            true,
        ),
        "dracula" => (
            [
                "282a36", "343746", "f8f8f2", "bdc1d0", "bd93f9", "44475a", "ff5555", "f1fa8c",
                "50fa7b",
            ],
            false,
        ),
        "solarized-dark" => (
            [
                "002b36", "073642", "eee8d5", "93a1a1", "2aa198", "174c56", "dc322f", "b58900",
                "859900",
            ],
            false,
        ),
        "terminal" => (
            [
                "101418", "1b232c", "e5edf4", "a8b9c9", "6ddbc1", "294053", "ff7f86", "f5c36c",
                "8edd99",
            ],
            false,
        ),
        _ => anyhow::bail!(
            "Unknown theme {name}; use :theme to list built-ins or a palette file path"
        ),
    };
    let v = values
        .iter()
        .map(|s| color(s))
        .collect::<Result<Vec<_>>>()?;
    Ok(Palette {
        name: name.into(),
        source: "Built-in".into(),
        background: v[0],
        surface: v[1],
        text: v[2],
        muted: v[3],
        accent: v[4],
        selection: v[5],
        error: v[6],
        warning: v[7],
        success: v[8],
        light,
    })
}

pub fn caelestia(raw: &str) -> Result<Palette> {
    let data: Value = serde_json::from_str(raw)?;
    let colours = data
        .get("colours")
        .context("Caelestia palette has no colours map")?;
    let light = data.get("mode").and_then(Value::as_str) == Some("light");
    let mut p = builtin(if light { "ytui-light" } else { "ytui-dark" })?;
    p.name = data
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Caelestia")
        .into();
    p.source = "Caelestia".into();
    let get = |names: &[&str]| -> Result<Rgb> {
        let value = names
            .iter()
            .find_map(|n| colours.get(n).and_then(Value::as_str))
            .context("Missing Caelestia semantic color")?;
        color(value)
    };
    p.background = get(&["surface", "background"])?;
    p.surface = get(&["surfaceContainer", "surfaceVariant", "surface"])?;
    p.text = get(&["onSurface", "onBackground"])?;
    p.muted = get(&["onSurfaceVariant", "onSurface"])?;
    p.accent = get(&["primary"])?;
    p.selection = get(&[
        "primaryContainer",
        "surfaceContainerHigh",
        "surfaceContainer",
    ])?;
    p.error = get(&["error"])?;
    Ok(p)
}
pub fn omarchy(raw: &str, legacy_light: bool) -> Result<Palette> {
    let data: toml::Value = toml::from_str(raw)?;
    let light = data.get("mode").and_then(toml::Value::as_str) == Some("light") || legacy_light;
    let mut p = builtin(if light { "ytui-light" } else { "ytui-dark" })?;
    p.name = "Omarchy".into();
    p.source = "Omarchy".into();
    let get = |keys: &[&str], fallback: Rgb| -> Result<Rgb> {
        keys.iter()
            .find_map(|k| data.get(*k).and_then(toml::Value::as_str))
            .map(color)
            .unwrap_or(Ok(fallback))
    };
    ensure!(
        data.get("background").is_some() && data.get("foreground").is_some(),
        "Omarchy palette lacks background/foreground"
    );
    p.background = get(&["background"], p.background)?;
    p.surface = get(&["lighter_background", "color0"], p.surface)?;
    p.text = get(&["foreground"], p.text)?;
    p.muted = get(&["dark_foreground", "muted", "color8"], p.muted)?;
    p.accent = get(&["accent", "blue", "color4"], p.accent)?;
    p.selection = get(&["selection", "color8"], p.selection)?;
    p.error = get(&["red", "color1"], p.error)?;
    p.warning = get(&["yellow", "color3"], p.warning)?;
    p.success = get(&["green", "color2"], p.success)?;
    Ok(p)
}
