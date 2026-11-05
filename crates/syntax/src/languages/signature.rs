//! Signature-help labels use UTF-16 offsets independently of document position encoding.

use serde_json::Value;
use std::io::{self, Write};
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Signature {
    pub label: String,
    pub parameter: Option<Range<usize>>,
    pub documentation: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SignatureHelp {
    pub signatures: Vec<Signature>,
    pub active: usize,
    pub raw: Arc<Value>,
}

fn utf16_byte(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += c.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}

const MAX_SIGNATURES: usize = 64;
const MAX_PARAMETERS: usize = 128;
const MAX_CONTEXT_BYTES: usize = 256 * 1024;

struct ContextBudget(usize);
impl Write for ContextBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.checked_add(bytes.len()).ok_or_else(|| io::Error::other("context too large"))?;
        if self.0 > MAX_CONTEXT_BYTES {
            return Err(io::Error::other("context too large"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn signature_help(value: &Value) -> Option<SignatureHelp> {
    let signatures = value["signatures"].as_array()?;
    if signatures.len() > MAX_SIGNATURES
        || signatures.iter().any(|s| s["parameters"].as_array().is_some_and(|p| p.len() > MAX_PARAMETERS))
        || serde_json::to_writer(ContextBudget(0), value).is_err()
    {
        return None;
    }
    let index = value["activeSignature"]
        .as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .filter(|&i| i < signatures.len())
        .unwrap_or(0);
    // Preserve protocol indices for the selected signature; malformed entries invalidate the reply.
    let parsed: Vec<_> = signatures.iter().map(|signature| parse_signature(signature, value)).collect::<Option<_>>()?;
    if parsed.is_empty() {
        return None;
    }
    Some(SignatureHelp { active: index, signatures: parsed, raw: Arc::new(value.clone()) })
}

fn parse_signature(signature: &Value, value: &Value) -> Option<Signature> {
    let label = signature["label"].as_str()?;
    if label.is_empty() || label.len() > 8192 {
        return None;
    }
    let parameters = signature["parameters"].as_array();
    let parameter = parameters.and_then(|parameters| {
        let active = signature
            .get("activeParameter")
            .unwrap_or(&value["activeParameter"])
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|&i| i < parameters.len())
            .unwrap_or(0);
        let mut from = label.find('(').map_or(0, |i| i + 1);
        let mut found = None;
        for parameter in parameters.iter().take(active + 1) {
            found = match &parameter["label"] {
                Value::String(text) => label[from..].find(text).map(|i| from + i..from + i + text.len()),
                Value::Array(offsets) if offsets.len() == 2 => {
                    let a =
                        offsets[0].as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| utf16_byte(label, n));
                    let b =
                        offsets[1].as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| utf16_byte(label, n));
                    a.zip(b).filter(|(a, b)| a <= b).map(|(a, b)| a..b)
                }
                _ => None,
            };
            if let Some(range) = &found {
                from = range.end;
            }
        }
        found
    });
    let documentation = signature
        .get("documentation")
        .and_then(|doc| doc.as_str().or_else(|| doc.get("value").and_then(Value::as_str)))
        .map(|text| text.chars().take(4096).collect());
    Some(Signature { label: label.into(), parameter, documentation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn active_parameter_offsets_are_utf16_and_signature_override_wins() {
        let value = json!({"activeParameter":0,"signatures":[{"label":"f(😀, 中)","activeParameter":1,
            "parameters":[{"label":[2,4]},{"label":[6,7]}]}]});
        let s = &signature_help(&value).unwrap().signatures[0];
        assert_eq!(s.parameter, Some(8..11));
        assert_eq!(&s.label[s.parameter.clone().unwrap()], "中");
        let malformed = json!({"signatures":[{"label":"f(😀)","parameters":[{"label":[3,4]}]}]});
        assert_eq!(signature_help(&malformed).unwrap().signatures[0].parameter, None);
        assert!(signature_help(&json!({"signatures":[]})).is_none());
    }

    #[test]
    fn retains_all_signatures_and_plain_documentation() {
        let help = signature_help(&json!({"activeSignature":1,"signatures":[
            {"label":"f(a)","documentation":"First"},
            {"label":"f(b)","documentation":{"kind":"plaintext","value":"Second"}}
        ]}))
        .unwrap();
        assert_eq!(help.signatures.len(), 2);
        assert_eq!(help.active, 1);
        assert_eq!(help.signatures[1].documentation.as_deref(), Some("Second"));
    }

    #[test]
    fn excessive_signature_context_is_rejected_before_retention() {
        assert!(signature_help(&json!({"signatures":[{"label":"f()","documentation":"x".repeat(300_000)}]})).is_none());
        assert!(signature_help(&json!({"signatures":vec![json!({"label":"f()"});65]})).is_none());
        assert!(signature_help(&json!({"signatures":[{"label":"f()","parameters":vec![json!({"label":"x"});129]}]}))
            .is_none());
        assert!(signature_help(&json!({"signatures":[{"label":"f()","unused":"中".repeat(100_000)}]})).is_none());
    }

    #[test]
    fn admitted_context_is_shared_and_preserves_utf16_protocol_offsets() {
        let value = json!({"activeSignature":1,"signatures":[{"label":"f(a)"},
            {"label":"f(😀, 中)","parameters":[{"label":[2,4]},{"label":[6,7]}]}]});
        let help = signature_help(&value).unwrap();
        let shared = help.raw.clone();
        assert!(Arc::ptr_eq(&help.raw, &shared));
        assert!(serde_json::to_vec(&*shared).unwrap().len() <= MAX_CONTEXT_BYTES);
        assert_eq!(shared["signatures"][1]["parameters"][1]["label"], json!([6, 7]));
        assert_eq!(help.active, 1);
    }

    #[test]
    fn string_parameters_and_invalid_active_indices_have_protocol_defaults() {
        let help = signature_help(&json!({"activeSignature":99,"activeParameter":99,"signatures":[
            {"label":"f(first: int, second: int)","parameters":[{"label":"first: int"},{"label":"second: int"}]}]}))
        .unwrap();
        assert_eq!(help.signatures[0].parameter, Some(2..12));
    }
}
