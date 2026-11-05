//! Configuration for one local stdio language server.

use serde::{de::Error, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(deserialize_with = "command")]
    pub command: Vec<String>,
    #[serde(deserialize_with = "initialization_options")]
    pub initialization_options: Value,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { command: Vec::new(), initialization_options: json!({}) }
    }
}

fn command<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let command = Vec::<String>::deserialize(d)?;
    if command.first().is_some_and(|program| program.trim().is_empty()) || command.iter().any(|s| s.contains('\0')) {
        return Err(D::Error::custom("command must be [] to disable, or a nonempty program and arguments without NUL"));
    }
    Ok(command)
}

/// A formatter's argument array, or `[]` to turn a language's formatter off.
#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct FormatterCommand(#[serde(deserialize_with = "command")] pub Vec<String>);

/// `formatters`: language id to command.
pub(crate) fn formatters<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, Vec<String>>, D::Error> {
    Ok(BTreeMap::<String, FormatterCommand>::deserialize(d)?.into_iter().map(|(id, c)| (id, c.0)).collect())
}

fn initialization_options<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
    let options = Value::deserialize(d)?;
    if !options.is_object() {
        return Err(D::Error::custom("initialization_options must be a JSON object"));
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_server_preferences_report_the_family_and_preserve_valid_siblings() {
        for invalid in [
            json!({"command":[""]}),
            json!({"command":["  "]}),
            json!({"command":["server\0bad"]}),
            json!({"command":["server","arg\0bad"]}),
            json!({"command":["server"],"initialization_options":[]}),
            json!({"command":["server"],"initialization_options":null}),
            json!({"command":["server"],"commmand":[]}),
        ] {
            let text = json!({"tab_size":3,"language_servers":{
                "working":{"command":["/user/working","--stdio"],"initialization_options":{"x":1}},
                "broken":invalid
            }})
            .to_string();
            let (settings, warning) = super::super::parse(&text).unwrap();
            let warning = warning.expect("invalid family needs an actionable warning");
            assert!(warning.contains("language_servers") && warning.contains("broken"), "{warning}");
            assert_eq!(settings.tab_size, 3);
            assert_eq!(settings.language_servers["working"].command, ["/user/working", "--stdio"]);
            assert!(!settings.language_servers.contains_key("broken"));
            assert!(serde_json::from_value::<ServerConfig>(invalid).is_err());
        }
        let valid = ServerConfig {
            command: vec!["/path with spaces/server".into(), "".into(), "$(touch nope)".into()],
            initialization_options: json!({}),
        };
        let value = serde_json::to_value(&valid).unwrap();
        assert_eq!(serde_json::from_value::<ServerConfig>(value).unwrap(), valid);
        assert_eq!(serde_json::from_value::<ServerConfig>(json!({"command":[]})).unwrap(), ServerConfig::default());
    }
}
