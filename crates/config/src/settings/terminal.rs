//! Application-wide terminal launch preferences.

use serde::{de::Error, Deserialize, Deserializer, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "RawTerminalSettings")]
pub struct TerminalSettings {
    pub shell: Option<String>,
    pub arguments: Option<Vec<String>>,
    pub shell_integration: bool,
    pub scrollback: usize,
    pub profiles: BTreeMap<String, TerminalProfile>,
    pub default_profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalProfile {
    #[serde(deserialize_with = "executable")]
    pub shell: String,
    #[serde(default = "login_arguments", deserialize_with = "profile_arguments")]
    pub arguments: Vec<String>,
    #[serde(default, deserialize_with = "directory")]
    pub directory: Option<PathBuf>,
    #[serde(default = "default_scrollback", deserialize_with = "scrollback")]
    pub scrollback: usize,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawTerminalSettings {
    #[serde(deserialize_with = "shell")]
    shell: Option<String>,
    #[serde(deserialize_with = "arguments")]
    arguments: Option<Vec<String>>,
    shell_integration: bool,
    #[serde(deserialize_with = "scrollback")]
    scrollback: usize,
    profiles: BTreeMap<String, TerminalProfile>,
    default_profile: Option<String>,
}

impl Default for RawTerminalSettings {
    fn default() -> Self {
        Self {
            shell: None,
            arguments: None,
            shell_integration: false,
            scrollback: default_scrollback(),
            profiles: BTreeMap::new(),
            default_profile: None,
        }
    }
}

impl Default for TerminalSettings {
    fn default() -> Self {
        RawTerminalSettings::default().try_into().unwrap()
    }
}

impl TryFrom<RawTerminalSettings> for TerminalSettings {
    type Error = String;
    fn try_from(value: RawTerminalSettings) -> Result<Self, String> {
        if value.profiles.len() > 32
            || value.profiles.keys().any(|name| {
                name.trim().is_empty()
                    || name.trim() != name
                    || name.chars().count() > 128
                    || name.chars().any(char::is_control)
            })
        {
            return Err(
                "Use at most 32 terminal profiles with trimmed names of 1–128 characters and no controls".into()
            );
        }
        if value.default_profile.as_ref().is_some_and(|name| !value.profiles.contains_key(name)) {
            return Err("default_profile must name an existing terminal profile".into());
        }
        Ok(Self {
            shell: value.shell,
            arguments: value.arguments,
            shell_integration: value.shell_integration,
            scrollback: value.scrollback,
            profiles: value.profiles,
            default_profile: value.default_profile,
        })
    }
}

fn default_scrollback() -> usize {
    10_000
}
fn login_arguments() -> Vec<String> {
    vec!["-l".into(), "-i".into()]
}

fn executable<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    shell(d)?.ok_or_else(|| D::Error::custom("profile shell must be an executable"))
}

fn profile_arguments<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    arguments(d)?.ok_or_else(|| D::Error::custom("profile arguments must be an array"))
}

fn directory<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
    let directory = Option::<String>::deserialize(d)?;
    if directory
        .as_ref()
        .is_some_and(|path| path.contains('\0') || path.len() > 4096 || !std::path::Path::new(path).is_absolute())
    {
        return Err(D::Error::custom("profile directory must be an absolute path without NUL, at most 4096 bytes"));
    }
    Ok(directory.map(PathBuf::from))
}

fn shell<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let shell = Option::<String>::deserialize(d)?;
    if shell.as_ref().is_some_and(|s| s.trim().is_empty() || s.contains('\0')) {
        return Err(D::Error::custom("shell must be a nonempty executable path without NUL"));
    }
    Ok(shell)
}

fn arguments<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    let arguments = Option::<Vec<String>>::deserialize(d)?;
    if arguments.as_ref().is_some_and(|args| {
        args.iter().any(|s| s.contains('\0')) || args.iter().map(String::len).sum::<usize>() > 32_768
    }) {
        return Err(D::Error::custom("terminal arguments must contain no NUL and total at most 32768 bytes"));
    }
    Ok(arguments)
}

fn scrollback<'de, D: Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    let value = usize::deserialize(d)?;
    if value > 100_000 {
        return Err(D::Error::custom("scrollback must be between 0 and 100000"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::super::parse;

    #[test]
    fn shell_integration_is_explicit_default_off_and_roundtrips() {
        let (default, warning) = parse("{}").unwrap();
        assert!(warning.is_none());
        assert_eq!(serde_json::to_value(default.terminal).unwrap()["shell_integration"], false);
        let (enabled, warning) = parse(r#"{"terminal":{"shell_integration":true}}"#).unwrap();
        assert!(warning.is_none(), "explicit integration must be accepted: {warning:?}");
        assert_eq!(serde_json::to_value(enabled.terminal).unwrap()["shell_integration"], true);
        let (invalid, warning) = parse(r#"{"terminal":{"shell_integration":"zsh"}}"#).unwrap();
        assert!(warning.is_some());
        assert_eq!(invalid.terminal, super::TerminalSettings::default());
    }

    #[test]
    fn terminal_settings_preserve_defaults_and_validate_before_launch() {
        let (default, warning) = parse("{}").unwrap();
        assert!(warning.is_none());
        assert_eq!(default.terminal.shell, None);
        assert_eq!(default.terminal.arguments, None);
        assert_eq!(default.terminal.scrollback, 10_000);
        let (custom, warning) =
            parse(r#"{"terminal":{"shell":"/bin/sh","arguments":["-c","printf '%s' '$HOME'"],"scrollback":0}}"#)
                .unwrap();
        assert!(warning.is_none());
        assert_eq!(custom.terminal.arguments.unwrap()[1], "printf '%s' '$HOME'");
        assert_eq!(custom.terminal.scrollback, 0);
        for invalid in [
            r#"{"shell":""}"#,
            r#"{"shell":"x\u0000y"}"#,
            r#"{"scrollback":100001}"#,
            r#"{"scrollback":-1}"#,
            r#"{"arguments":["x\u0000y"]}"#,
        ] {
            let (settings, warning) = parse(&format!("{{\"terminal\":{invalid}}}")).unwrap();
            assert!(warning.unwrap().contains("terminal"));
            assert_eq!(settings.terminal.scrollback, 10000);
        }
    }

    #[test]
    fn named_terminal_profiles_roundtrip_literal_launch_options() {
        let (settings, warning) = parse(r#"{"terminal":{
            "profiles":{"Build café":{"shell":"/bin/sh","arguments":["-c","printf %s \"$1\"","orlo","literal $HOME with spaces", ""],"directory":"/tmp/build dir","scrollback":2}},
            "default_profile":"Build café"
        }}"#).unwrap();
        assert!(warning.is_none(), "named profiles must be accepted: {warning:?}");
        let encoded = serde_json::to_value(&settings.terminal).unwrap();
        assert_eq!(encoded["default_profile"], "Build café");
        assert_eq!(encoded["profiles"]["Build café"]["arguments"][3], "literal $HOME with spaces");
        assert_eq!(encoded["profiles"]["Build café"]["arguments"][4], "");
        assert_eq!(encoded["profiles"]["Build café"]["directory"], "/tmp/build dir");
        assert_eq!(encoded["profiles"]["Build café"]["scrollback"], 2);
        let (_, warning) = parse(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(warning.is_none());
    }

    #[test]
    fn named_profiles_validate_names_options_and_default_references() {
        let profile = serde_json::json!({"shell":"/bin/sh"});
        for invalid in [
            serde_json::json!({"profiles":{"bad\nname":profile}}),
            serde_json::json!({"profiles":{" name ":profile}}),
            serde_json::json!({"profiles":{ "":profile}}),
            serde_json::json!({"profiles":{ "ok":{"shell":""}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":null}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","directory":"relative"}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","directory":"/x\u{0}y"}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","arguments":null}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","arguments":["x\u{0}y"]}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","arguments":["x".repeat(32769)]}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","scrollback":-1}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","scrollback":100001}}}),
            serde_json::json!({"profiles":{ "ok":{"shell":"/bin/sh","unknown":true}}}),
            serde_json::json!({"default_profile":"missing"}),
        ] {
            let (settings, warning) = parse(&serde_json::json!({"terminal":invalid}).to_string()).unwrap();
            assert!(warning.is_some(), "invalid options must warn: {invalid}");
            assert_eq!(settings.terminal, super::TerminalSettings::default());
        }
        let too_many: serde_json::Map<_, _> = (0..33).map(|i| (i.to_string(), profile.clone())).collect();
        assert!(parse(&serde_json::json!({"terminal":{"profiles":too_many}}).to_string()).unwrap().1.is_some());
        let (settings, warning) = parse(r#"{"terminal":{"profiles":{"Plain":{"shell":"/bin/sh"}}}}"#).unwrap();
        assert!(warning.is_none());
        let plain = &settings.terminal.profiles["Plain"];
        assert_eq!(plain.arguments, ["-l", "-i"]);
        assert_eq!(plain.directory, None);
        assert_eq!(plain.scrollback, 10000);
    }
}
