//! Lossless JSON whitespace tools: validate syntax without converting tokens to values.

pub const MAX_INPUT: usize = 16 * 1024 * 1024;
pub const MAX_OUTPUT: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Format,
    Minify,
    Validate,
}

/// `None` means validation only. Tokens are never deserialized into numbers, strings or maps.
pub fn process(text: &str, operation: Operation) -> Result<Option<String>, String> {
    if text.len() > MAX_INPUT {
        return Err("JSON input exceeds the 16 MiB limit".into());
    }
    // IgnoredAny checks syntax iteratively without rounding numbers or allocating a value tree.
    serde_json::from_str::<serde::de::IgnoredAny>(text).map_err(|error| error.to_string())?;
    if operation == Operation::Validate {
        return Ok(None);
    }
    let mut out = String::with_capacity(text.len());
    let mut tokens = tokens(text).peekable();
    let mut depth = 0;
    let mut previous = "";
    while let Some(token) = tokens.next() {
        if operation == Operation::Minify {
            push(&mut out, token)?;
            continue;
        }
        match token {
            "{" | "[" => {
                push(&mut out, token)?;
                depth += 1;
                let close = if token == "{" { "}" } else { "]" };
                if tokens.peek().copied() != Some(close) {
                    newline(&mut out, depth)?;
                }
            }
            "}" | "]" => {
                depth -= 1;
                let open = if token == "}" { "{" } else { "[" };
                if previous != open {
                    newline(&mut out, depth)?;
                }
                push(&mut out, token)?;
            }
            "," => {
                push(&mut out, token)?;
                newline(&mut out, depth)?;
            }
            ":" => push(&mut out, ": ")?,
            _ => push(&mut out, token)?,
        }
        previous = token;
    }
    Ok(Some(out))
}

fn push(out: &mut String, text: &str) -> Result<(), String> {
    if text.len() > MAX_OUTPUT - out.len() {
        return Err("JSON output exceeds the 32 MiB limit".into());
    }
    out.push_str(text);
    Ok(())
}

fn newline(out: &mut String, depth: usize) -> Result<(), String> {
    if 1 + depth * 2 > MAX_OUTPUT - out.len() {
        return Err("JSON output exceeds the 32 MiB limit".into());
    }
    out.push('\n');
    out.extend(std::iter::repeat_n("  ", depth));
    Ok(())
}

/// Input was validated first, so a token is punctuation, an intact string, or an ASCII literal.
fn tokens(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        text = text.trim_start_matches([' ', '\t', '\r', '\n']);
        let bytes = text.as_bytes();
        let first = *bytes.first()?;
        let end = match first {
            b'"' => {
                let mut at = 1;
                while at < bytes.len() {
                    match bytes[at] {
                        b'\\' => at += 2,
                        b'"' => {
                            at += 1;
                            break;
                        }
                        _ => at += 1,
                    }
                }
                at
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => 1,
            // a literal runs to white space or punctuation
            _ => (bytes.iter()).position(|b| b" \t\r\n{}[],:".contains(b)).unwrap_or(bytes.len()),
        };
        let (token, remaining) = text.split_at(end);
        text = remaining;
        Some(token)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_format_uses_two_spaces_and_keeps_empty_containers_compact() {
        let input = r#" { "z" : [1, {"x": true}], "a": {}, "b": [] } "#;
        let want = "{\n  \"z\": [\n    1,\n    {\n      \"x\": true\n    }\n  ],\n  \"a\": {},\n  \"b\": []\n}";
        assert_eq!(process(input, Operation::Format).unwrap(), Some(want.into()));
    }

    #[test]
    fn json_whitespace_tools_preserve_numeric_lexemes_key_order_duplicates_and_escapes() {
        let input = r#"{ "z": 1234567890123456789012345678901234567890, "a": 1e+999999, "z": -0.00E-0001, "s": "\u0061\/\"\\\n café" }"#;
        let compact =
            r#"{"z":1234567890123456789012345678901234567890,"a":1e+999999,"z":-0.00E-0001,"s":"\u0061\/\"\\\n café"}"#;
        assert_eq!(process(input, Operation::Minify).unwrap(), Some(compact.into()));
        let pretty = process(input, Operation::Format).unwrap().unwrap();
        assert_eq!(process(&pretty, Operation::Minify).unwrap(), Some(compact.into()));
        assert_eq!(process(input, Operation::Validate).unwrap(), None);
    }

    #[test]
    fn json_whitespace_tools_support_scalar_values_and_do_not_strip_string_spaces() {
        for (input, want) in [
            (" \r\n true\t", "true"),
            (" null ", "null"),
            (" -0 ", "-0"),
            (" 1.20E+09 ", "1.20E+09"),
            (r#" " a b [{}],: " "#, r#"" a b [{}],: ""#),
            (" [ ] ", "[]"),
            (" { } ", "{}"),
        ] {
            for operation in [Operation::Format, Operation::Minify] {
                assert_eq!(process(input, operation).unwrap(), Some(want.into()));
            }
        }
    }

    #[test]
    fn json_validation_rejects_malformed_and_trailing_tokens_with_line_and_column() {
        for input in ["", "  ", "01", "[1,]", "[1 2]", "true false", "{\n\"a\": }", r#""\q""#, "1e+"] {
            for operation in [Operation::Format, Operation::Minify, Operation::Validate] {
                let error = process(input, operation).unwrap_err();
                assert!(error.contains("line "), "{input:?}: {error}");
                assert!(error.contains("column "), "{input:?}: {error}");
            }
        }
    }

    #[test]
    fn json_input_and_expanded_output_are_bounded() {
        let too_big = format!("\"{}\"", "x".repeat(MAX_INPUT));
        assert!(process(&too_big, Operation::Validate).unwrap_err().contains("limit"));
        let nested = format!("{}{}0{}", "[".repeat(100), "0,".repeat(170_000), "]".repeat(100));
        assert!(nested.len() < MAX_INPUT);
        assert!(process(&nested, Operation::Format).unwrap_err().contains("limit"));
        assert!(process(&nested, Operation::Minify).unwrap().unwrap().len() < MAX_OUTPUT);
    }
}
