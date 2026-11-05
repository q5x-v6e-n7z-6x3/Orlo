//! Language-server defaults and parsing shared by the editor and workspace.

use crate::lsp::{self, Encoding, Position, Span};
use orlo_editor::text::{CharOffset, Coords};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

// One catalog keeps the public variants, enumeration and protocol/server IDs in step.
macro_rules! languages {
    ($($variant:ident => ($id:literal, $server:literal),)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Language { $($variant,)* }

        impl Language {
            pub const ALL: [Self; [$(stringify!($variant)),*].len()] = [$(Self::$variant,)*];

            pub fn id(self) -> &'static str {
                match self { $(Self::$variant => $id,)* }
            }

            pub fn server(self) -> &'static str {
                match self { $(Self::$variant => $server,)* }
            }
        }
    };
}

languages! {
    Rust => ("rust", "rust"),
    Go => ("go", "go"),
    JavaScript => ("javascript", "typescript"),
    Jsx => ("javascriptreact", "typescript"),
    TypeScript => ("typescript", "typescript"),
    Tsx => ("typescriptreact", "typescript"),
    Python => ("python", "python"),
    C => ("c", "clangd"),
    Cpp => ("cpp", "clangd"),
    CSharp => ("csharp", "csharp"),
    Java => ("java", "java"),
    Php => ("php", "php"),
    Ruby => ("ruby", "ruby"),
    Swift => ("swift", "swift"),
    Kotlin => ("kotlin", "kotlin"),
    Bash => ("shellscript", "bash"),
    Hcl => ("hcl", "hcl"),
    Terraform => ("terraform", "terraform"),
    TerraformVars => ("terraform-vars", "terraform"),
    TerraformStack => ("terraform-stack", "terraform"),
    TerraformDeploy => ("terraform-deploy", "terraform"),
    TerraformSearch => ("terraform-search", "terraform"),
    TerraformPolicy => ("terraform-policy", "terraform"),
    TerraformPolicyTest => ("terraform-policytest", "terraform"),
}

impl Language {
    /// The language folder id; JSX, TSX and Bash differ from their LSP ids.
    pub fn registry_id(self) -> &'static str {
        match self {
            Self::Jsx => "jsx",
            Self::Tsx => "tsx",
            Self::Bash => "bash",
            _ => self.id(),
        }
    }

    pub fn from_registry_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.registry_id() == id)
    }

    pub fn detect(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        match name {
            "Gemfile" | "Rakefile" | "Guardfile" => return Some(Self::Ruby),
            ".bashrc" | ".bash_profile" | ".bash_login" | ".profile" => return Some(Self::Bash),
            _ => {}
        }
        for (suffix, language) in [
            (".tfcomponent.hcl", Self::TerraformStack),
            (".tfdeploy.hcl", Self::TerraformDeploy),
            (".tfquery.hcl", Self::TerraformSearch),
            (".policy.hcl", Self::TerraformPolicy),
            (".policytest.hcl", Self::TerraformPolicyTest),
        ] {
            if name.ends_with(suffix) {
                return Some(language);
            }
        }
        Some(match path.extension()?.to_str()? {
            "rs" => Self::Rust,
            "go" => Self::Go,
            "js" | "mjs" | "cjs" => Self::JavaScript,
            "jsx" => Self::Jsx,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "py" | "pyw" | "pyi" => Self::Python,
            "c" | "h" => Self::C,
            "C" | "cc" | "cp" | "cpp" | "cxx" | "c++" | "hh" | "hpp" | "hxx" | "h++" => Self::Cpp,
            "cs" | "csx" => Self::CSharp,
            "java" => Self::Java,
            "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "php8" => Self::Php,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            "swift" => Self::Swift,
            "kt" | "kts" => Self::Kotlin,
            "sh" | "bash" => Self::Bash,
            "hcl" => Self::Hcl,
            "tf" => Self::Terraform,
            "tfvars" => Self::TerraformVars,
            _ => return None,
        })
    }

    pub fn markers(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["Cargo.toml"],
            Self::Go => &["go.work", "go.mod"],
            Self::Python => &["pyproject.toml", "setup.py", "setup.cfg", "requirements.txt"],
            Self::JavaScript | Self::Jsx | Self::TypeScript | Self::Tsx => {
                &["tsconfig.json", "jsconfig.json", "package.json"]
            }
            Self::C | Self::Cpp => {
                &["compile_commands.json", "compile_flags.txt", "CMakeLists.txt", "meson.build", "Makefile"]
            }
            Self::CSharp => {
                &["global.json", "Directory.Build.props", "Directory.Build.targets", "NuGet.Config", "nuget.config"]
            }
            Self::Java | Self::Kotlin => {
                &["settings.gradle.kts", "settings.gradle", "build.gradle.kts", "build.gradle", "pom.xml"]
            }
            Self::Php => &["composer.json"],
            Self::Ruby => &["Gemfile", "Rakefile"],
            Self::Swift => &["Package.swift", "compile_commands.json"],
            Self::Bash => &[".shellcheckrc", ".bash-language-server.json"],
            Self::Hcl => &[],
            Self::Terraform
            | Self::TerraformVars
            | Self::TerraformStack
            | Self::TerraformDeploy
            | Self::TerraformSearch
            | Self::TerraformPolicy
            | Self::TerraformPolicyTest => &[".terraform.lock.hcl", "main.tf", "terraform.tf"],
        }
    }

    pub fn is_project_root(self, path: &Path) -> bool {
        self.markers().iter().any(|marker| path.join(marker).is_file())
            || self == Self::CSharp
                && std::fs::read_dir(path).is_ok_and(|entries| {
                    entries.flatten().any(|entry| {
                        entry
                            .path()
                            .extension()
                            .is_some_and(|ext| matches!(ext.to_str(), Some("sln" | "slnx" | "csproj")))
                            && entry.path().is_file()
                    })
                })
    }
}

/// A runtime protocol identity; parser selection continues to use the bundled `Language`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerLanguage {
    pub id: String,
    pub server: String,
    markers: Vec<String>,
}

impl ServerLanguage {
    /// A file declaration has already resolved to a catalog id or available syntax name.
    pub fn declared(registry: &crate::registry::Registry, choice: &str) -> Option<Self> {
        if let Some(language) = registry.get(choice) {
            return Self::manifest(language);
        }
        Language::from_registry_id(choice).map(Self::bundled).or_else(|| Self::named(registry, choice))
    }

    pub fn detect(registry: &crate::registry::Registry, path: &Path, head: &str) -> Option<Self> {
        if let Some(language) = registry.detect(Some(path), head) {
            return Self::manifest(language);
        }
        let bundled = Language::detect(path)?;
        // A valid same-ID folder replaces the bundled definition, including its file claims.
        if registry.get(bundled.registry_id()).is_some() {
            return None;
        }
        Some(Self::bundled(bundled))
    }

    pub fn named(registry: &crate::registry::Registry, name: &str) -> Option<Self> {
        if let Some(language) = registry.named(name) {
            return Self::manifest(language);
        }
        let bundled = crate::treesitter::named(name)?;
        registry.get(bundled.registry_id()).map_or_else(|| Some(Self::bundled(bundled)), |l| Self::manifest(l))
    }

    fn manifest(language: &crate::registry::LanguageDef) -> Option<Self> {
        let lsp = language.manifest.lsp.as_ref()?;
        Some(Self {
            id: lsp.language_id.clone(),
            server: language.manifest.server()?.into(),
            markers: lsp.root_markers.clone(),
        })
    }

    fn bundled(bundled: Language) -> Self {
        let mut markers: Vec<String> = bundled.markers().iter().map(|s| (*s).into()).collect();
        if bundled == Language::CSharp {
            markers.extend(["*.sln", "*.slnx", "*.csproj"].map(String::from));
        }
        Self { id: bundled.id().into(), server: bundled.server().into(), markers }
    }

    pub fn is_project_root(&self, path: &Path) -> bool {
        self.markers.iter().filter(|m| !m.starts_with('*')).any(|m| path.join(m).is_file())
            || self.markers.iter().filter_map(|m| m.strip_prefix('*')).any(|suffix| {
                std::fs::read_dir(path).is_ok_and(|entries| {
                    entries.flatten().any(|entry| {
                        entry.file_name().to_str().is_some_and(|name| name.ends_with(suffix)) && entry.path().is_file()
                    })
                })
            })
    }
}

/// Manifest family defaults overlay bundled defaults; Preferences overlay the returned map.
/// For shared families the first manifest by ID supplying each field wins.
pub fn runtime_servers(registry: &crate::registry::Registry) -> BTreeMap<String, ServerConfig> {
    let mut servers = default_servers();
    let mut commands = std::collections::HashSet::new();
    let mut options = std::collections::HashSet::new();
    for language in registry.languages() {
        let Some(lsp) = &language.manifest.lsp else { continue };
        let family = language.manifest.server().unwrap();
        let server = servers.entry(family.into()).or_default();
        if let Some(command) = &lsp.command {
            if commands.insert(family) {
                server.command = command.clone();
            }
        }
        if let Some(init) = &lsp.initialization_options {
            if options.insert(family) {
                server.initialization_options = init.clone();
            }
        }
    }
    servers
}

pub use orlo_config::settings::ServerConfig;

pub fn default_servers() -> BTreeMap<String, ServerConfig> {
    [
        ("rust", vec!["rust-analyzer"]),
        ("go", vec!["gopls"]),
        ("typescript", vec!["typescript-language-server", "--stdio"]),
        ("python", vec!["pylsp"]),
        ("clangd", vec!["clangd"]),
        ("csharp", vec!["csharp-ls"]),
        ("java", vec!["jdtls"]),
        ("php", vec!["intelephense", "--stdio"]),
        ("ruby", vec!["ruby-lsp"]),
        ("swift", vec!["xcrun", "sourcekit-lsp"]),
        ("kotlin", vec!["kotlin-lsp", "--stdio"]),
        ("bash", vec!["bash-language-server", "start"]),
        ("terraform", vec!["terraform-ls", "serve"]),
        // Generic HCL needs a server for its dialect; terraform-ls explicitly rejects it.
        ("hcl", vec![]),
    ]
    .into_iter()
    .map(|(key, cmd)| {
        (
            key.into(),
            ServerConfig {
                command: cmd.into_iter().map(String::from).collect(),
                initialization_options: if key == "go" { json!({"semanticTokens":true}) } else { json!({}) },
            },
        )
    })
    .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: u64,
    pub message: String,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticSpan {
    pub range: Range<usize>,
    pub kind: String,
}

pub fn diagnostics(text: Coords, values: &Value, encoding: Encoding) -> Vec<Diagnostic> {
    values
        .as_array()
        .into_iter()
        .flatten()
        .take(10_000)
        .filter_map(|d| {
            Some(Diagnostic {
                range: lsp::char_range(text, &d["range"], encoding).ok()?,
                severity: d["severity"].as_u64().unwrap_or(1),
                message: d["message"].as_str()?.chars().take(8192).collect(),
                raw: d.clone(),
            })
        })
        .collect()
}

pub fn completion_edits(
    text: Coords,
    cursor: usize,
    item: &Value,
    encoding: Encoding,
) -> Result<Vec<(Range<usize>, String)>, String> {
    let primary = if item["textEdit"].is_object() {
        let edit = &item["textEdit"];
        json!({"range": edit.get("range").or_else(||edit.get("replace"))
            .ok_or("Completion has no replacement range")?, "newText":edit["newText"]})
    } else {
        let rope = text.rope();
        let cursor = cursor.min(rope.len_chars());
        let mut start = cursor;
        while start > 0 && (rope.char(start - 1).is_alphanumeric() || rope.char(start - 1) == '_') {
            start -= 1;
        }
        json!({"range":lsp::span(text, CharOffset(start)..CharOffset(cursor), encoding), "newText":
            item.get("insertText").or_else(||item.get("textEditText")).or_else(||item.get("label"))
                .and_then(Value::as_str).ok_or("Completion has no text")?})
    };
    let mut edits = vec![primary];
    if let Some(additional) = item.get("additionalTextEdits") {
        edits.extend(additional.as_array().ok_or("Invalid additional completion edits")?.iter().cloned());
    }
    lsp::text_edits(text, &Value::Array(edits), encoding)
}

/// A completion item's kind (LSP numbers them from 1), named for the list's badge; empty when the
/// server gives none.
pub fn completion_kind(item: &Value) -> &'static str {
    const KINDS: [&str; 25] = [
        "text",
        "method",
        "function",
        "constructor",
        "field",
        "variable",
        "class",
        "interface",
        "module",
        "property",
        "unit",
        "value",
        "enum",
        "keyword",
        "snippet",
        "color",
        "file",
        "reference",
        "folder",
        "enum member",
        "constant",
        "struct",
        "event",
        "operator",
        "type parameter",
    ];
    let n = item["kind"].as_u64().and_then(|k| usize::try_from(k).ok());
    n.and_then(|k| KINDS.get(k.checked_sub(1)?)).copied().unwrap_or("")
}

/// A definition or references reply as (URI, span); its positions are in the server's encoding
/// and its text, converted only once that document is open.
pub fn locations(value: &Value) -> Result<Vec<(String, Span)>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value.as_array().map_or_else(|| vec![value], |v| v.iter().collect());
    values
        .into_iter()
        .map(|v| {
            let range = v.get("range").or_else(|| v.get("targetSelectionRange")).ok_or("Invalid location range")?;
            for endpoint in ["start", "end"] {
                for component in ["line", "character"] {
                    if range[endpoint][component].as_u64().is_none_or(|n| n > i32::MAX as u64) {
                        return Err("Invalid location position".into());
                    }
                }
            }
            Ok((
                v.get("uri")
                    .or_else(|| v.get("targetUri"))
                    .and_then(Value::as_str)
                    .ok_or("Invalid location URI")?
                    .to_string(),
                Span::parse(range)?,
            ))
        })
        .collect()
}

pub fn hover_text(value: &Value) -> String {
    fn text(v: &Value) -> String {
        if let Some(s) = v.as_str() {
            return s.into();
        }
        if let Some(a) = v.as_array() {
            return a.iter().map(text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
        }
        v["value"].as_str().unwrap_or("").into()
    }
    text(&value["contents"]).chars().take(64 * 1024).collect()
}

pub fn semantic_tokens(
    text: Coords,
    value: &Value,
    legend: &Value,
    encoding: Encoding,
) -> Result<Vec<SemanticSpan>, String> {
    let data = value["data"].as_array().ok_or("Invalid semantic tokens")?;
    if data.len() % 5 != 0 || data.len() > 500_000 {
        return Err("Invalid semantic token count".into());
    }
    let (mut line, mut col) = (0usize, 0usize);
    let mut out = Vec::new();
    for token in data.as_chunks::<5>().0 {
        let numbers: Vec<_> = token
            .iter()
            .map(|n| n.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("Invalid semantic token"))
            .collect::<Result<_, _>>()?;
        line = line.checked_add(numbers[0]).ok_or("Semantic line overflow")?;
        col = if numbers[0] == 0 { col.checked_add(numbers[1]).ok_or("Semantic column overflow")? } else { numbers[1] };
        let end = col.checked_add(numbers[2]).ok_or("Semantic length overflow")?;
        let kind = legend["tokenTypes"].get(numbers[3]).and_then(Value::as_str).ok_or("Unknown semantic token type")?;
        let span = Span { start: Position::new(line, col), end: Position::new(line, end) };
        let r = lsp::range(text, span, encoding)?;
        let r = r.start.0..r.end.0;
        if out.last().is_some_and(|s: &SemanticSpan| s.range.end > r.start) {
            return Err("Overlapping semantic tokens".into());
        }
        out.push(SemanticSpan { range: r, kind: kind.into() });
    }
    Ok(out)
}

pub fn semantic_scope(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "namespace" | "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => "entity.name.type",
        "parameter" => "variable.parameter",
        "variable" => "variable.other",
        "property" => "variable.member",
        "enumMember" => "constant.other",
        "function" | "method" => "entity.name.function",
        "macro" => "support.macro",
        "keyword" | "modifier" => "keyword",
        "comment" => "comment",
        "string" => "string",
        "number" => "constant.numeric",
        "operator" => "keyword.operator",
        "decorator" => "variable.annotation",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::Encoding;
    use ropey::Rope;
    use serde_json::json;
    use std::path::Path;

    fn runtime_registry(name: &str, definitions: &[Value]) -> crate::registry::Registry {
        let root = std::env::temp_dir().join(format!("orlo-runtime-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folders = definitions
            .iter()
            .map(|definition| {
                let folder = root.join(definition["id"].as_str().unwrap());
                std::fs::create_dir_all(&folder).unwrap();
                std::fs::write(folder.join("language.json"), definition.to_string()).unwrap();
                (crate::registry::Origin::User, folder)
            })
            .collect::<Vec<_>>();
        let (registry, errors) = crate::registry::Registry::build(folders);
        assert!(errors.is_empty(), "{errors:?}");
        std::fs::remove_dir_all(root).unwrap();
        registry
    }

    #[test]
    fn runtime_language_claims_replace_bundled_detection_without_affecting_other_ids() {
        let registry = runtime_registry(
            "claims",
            &[
                json!({"schema":1,"id":"rust","name":"Custom Rust","scope":"source.custom",
                "files":{"extensions":["rux"],"first_line":"^#!.*custom-rust"},
                "lsp":{"language_id":"custom-rust","command":["custom-rust-lsp"]}}),
                json!({"schema":1,"id":"plain","name":"Plain Claim","scope":"text.plain",
                "files":{"extensions":["py"]}}),
            ],
        );
        assert!(ServerLanguage::detect(&registry, Path::new("a.rs"), "").is_none());
        assert!(ServerLanguage::detect(&registry, Path::new("a.py"), "").is_none());
        for (path, head) in [("a.RUX", ""), ("run", "#!/bin/custom-rust\n😀")] {
            let language = ServerLanguage::detect(&registry, Path::new(path), head).unwrap();
            assert_eq!((language.id.as_str(), language.server.as_str()), ("custom-rust", "rust"));
        }
        let go = ServerLanguage::detect(&registry, Path::new("a.go"), "").unwrap();
        assert_eq!((go.id.as_str(), go.server.as_str()), ("go", "go"));
        assert_eq!(ServerLanguage::named(&registry, "Rust").unwrap().id, "custom-rust");
        assert!(ServerLanguage::named(&registry, "Plain Claim").is_none());
        assert!(ServerLanguage::named(&registry, "Plain Text").is_none());
    }

    #[test]
    fn runtime_language_family_defaults_are_deterministic_and_keep_bundled_defaults() {
        let registry = runtime_registry(
            "defaults",
            &[
                json!({"schema":1,"id":"beta","name":"Beta","scope":"source.beta",
                "lsp":{"language_id":"beta","server":"shared","command":["second"],
                    "initialization_options":{"which":"second"}}}),
                json!({"schema":1,"id":"alpha","name":"Alpha","scope":"source.alpha",
                "lsp":{"language_id":"alpha","server":"shared","command":["first"]}}),
                json!({"schema":1,"id":"go-extra","name":"Go Extra","scope":"source.go",
                "lsp":{"language_id":"go-extra","server":"go","command":["custom-gopls"]}}),
            ],
        );
        let servers = runtime_servers(&registry);
        assert_eq!(servers["shared"].command, ["first"]);
        assert_eq!(servers["shared"].initialization_options, json!({"which":"second"}));
        assert_eq!(servers["go"].command, ["custom-gopls"]);
        assert_eq!(servers["go"].initialization_options, json!({"semanticTokens":true}));
        assert_eq!(servers["typescript"].command, ["typescript-language-server", "--stdio"]);
    }

    #[test]
    fn language_variants_use_the_right_server_family() {
        for (file, id, server) in [
            ("a.rs", "rust", "rust"),
            ("a.go", "go", "go"),
            ("a.jsx", "javascriptreact", "typescript"),
            ("a.mjs", "javascript", "typescript"),
            ("a.tsx", "typescriptreact", "typescript"),
            ("a.cts", "typescript", "typescript"),
            ("a.py", "python", "python"),
        ] {
            let language = Language::detect(Path::new(file)).unwrap();
            assert_eq!((language.id(), language.server()), (id, server));
        }
        assert!(Language::detect(Path::new("a.txt")).is_none());
        assert_eq!(default_servers()["python"].command, vec!["pylsp"]);
    }

    #[test]
    fn popular_languages_have_detected_ids_and_configured_server_families() {
        for (file, id, server) in [
            ("Main.java", "java", "java"),
            ("main.c", "c", "clangd"),
            ("main.h", "c", "clangd"),
            ("main.cpp", "cpp", "clangd"),
            ("main.hpp", "cpp", "clangd"),
            ("main.C", "cpp", "clangd"),
            ("Main.cs", "csharp", "csharp"),
            ("main.php", "php", "php"),
            ("main.rb", "ruby", "ruby"),
            ("Gemfile", "ruby", "ruby"),
            ("Rakefile", "ruby", "ruby"),
            ("main.swift", "swift", "swift"),
            ("main.kt", "kotlin", "kotlin"),
            ("build.gradle.kts", "kotlin", "kotlin"),
            ("main.sh", "shellscript", "bash"),
            (".bashrc", "shellscript", "bash"),
            ("main.hcl", "hcl", "hcl"),
            ("main.tf", "terraform", "terraform"),
            ("main.tfvars", "terraform-vars", "terraform"),
            ("main.tfcomponent.hcl", "terraform-stack", "terraform"),
            ("main.tfdeploy.hcl", "terraform-deploy", "terraform"),
            ("main.tfquery.hcl", "terraform-search", "terraform"),
            ("main.policy.hcl", "terraform-policy", "terraform"),
            ("main.policytest.hcl", "terraform-policytest", "terraform"),
        ] {
            let language = Language::detect(Path::new(file)).unwrap_or_else(|| panic!("missing language for {file}"));
            assert_eq!((language.id(), language.server()), (id, server), "{file}");
            assert_eq!(default_servers()[server].command.is_empty(), server == "hcl", "{server}");
        }
        assert!(Language::detect(Path::new("main.zsh")).is_none());
        assert!(Language::detect(Path::new("main.tf.json")).is_none());
    }

    #[test]
    fn every_variant_has_its_own_registry_id() {
        for language in Language::ALL {
            assert_eq!(Language::from_registry_id(language.registry_id()), Some(language));
        }
        assert_eq!(Language::from_registry_id("javascriptreact"), None);
        assert_eq!(Language::from_registry_id("pony"), None);
    }

    /// Every file claim `Language::detect` makes, and near misses it rejects.
    const SAMPLES: &[(&str, Option<Language>)] = &[
        ("main.rs", Some(Language::Rust)),
        ("main.go", Some(Language::Go)),
        ("main.js", Some(Language::JavaScript)),
        ("main.mjs", Some(Language::JavaScript)),
        ("main.cjs", Some(Language::JavaScript)),
        ("main.jsx", Some(Language::Jsx)),
        ("main.ts", Some(Language::TypeScript)),
        ("main.mts", Some(Language::TypeScript)),
        ("main.cts", Some(Language::TypeScript)),
        ("main.d.ts", Some(Language::TypeScript)),
        ("main.tsx", Some(Language::Tsx)),
        ("main.py", Some(Language::Python)),
        ("main.pyw", Some(Language::Python)),
        ("main.pyi", Some(Language::Python)),
        ("main.c", Some(Language::C)),
        ("main.h", Some(Language::C)),
        ("main.C", Some(Language::Cpp)),
        ("main.cc", Some(Language::Cpp)),
        ("main.cp", Some(Language::Cpp)),
        ("main.cpp", Some(Language::Cpp)),
        ("main.cxx", Some(Language::Cpp)),
        ("main.c++", Some(Language::Cpp)),
        ("main.hh", Some(Language::Cpp)),
        ("main.hpp", Some(Language::Cpp)),
        ("main.hxx", Some(Language::Cpp)),
        ("main.h++", Some(Language::Cpp)),
        ("Main.cs", Some(Language::CSharp)),
        ("Main.csx", Some(Language::CSharp)),
        ("Main.java", Some(Language::Java)),
        ("main.php", Some(Language::Php)),
        ("main.phtml", Some(Language::Php)),
        ("main.php3", Some(Language::Php)),
        ("main.php4", Some(Language::Php)),
        ("main.php5", Some(Language::Php)),
        ("main.php7", Some(Language::Php)),
        ("main.php8", Some(Language::Php)),
        ("main.rb", Some(Language::Ruby)),
        ("tasks.rake", Some(Language::Ruby)),
        ("orlo.gemspec", Some(Language::Ruby)),
        ("Gemfile", Some(Language::Ruby)),
        ("Rakefile", Some(Language::Ruby)),
        ("Guardfile", Some(Language::Ruby)),
        ("main.swift", Some(Language::Swift)),
        ("main.kt", Some(Language::Kotlin)),
        ("build.gradle.kts", Some(Language::Kotlin)),
        ("main.sh", Some(Language::Bash)),
        ("main.bash", Some(Language::Bash)),
        (".bashrc", Some(Language::Bash)),
        (".bash_profile", Some(Language::Bash)),
        (".bash_login", Some(Language::Bash)),
        (".profile", Some(Language::Bash)),
        ("main.hcl", Some(Language::Hcl)),
        ("main.tf", Some(Language::Terraform)),
        ("main.tfvars", Some(Language::TerraformVars)),
        ("main.tfcomponent.hcl", Some(Language::TerraformStack)),
        (".tfcomponent.hcl", Some(Language::TerraformStack)),
        ("main.tfdeploy.hcl", Some(Language::TerraformDeploy)),
        (".tfdeploy.hcl", Some(Language::TerraformDeploy)),
        ("main.tfquery.hcl", Some(Language::TerraformSearch)),
        (".tfquery.hcl", Some(Language::TerraformSearch)),
        ("main.policy.hcl", Some(Language::TerraformPolicy)),
        (".policy.hcl", Some(Language::TerraformPolicy)),
        ("main.policytest.hcl", Some(Language::TerraformPolicyTest)),
        (".policytest.hcl", Some(Language::TerraformPolicyTest)),
        ("main.rs.bak", None),
        ("main.tf.json", None),
        ("main.zsh", None),
        ("main.txt", None),
        ("Gemfile.lock", None),
        (".rs", None),
        ("rs", None),
    ];

    #[test]
    fn bundled_manifests_reproduce_every_language_variant() {
        use crate::manifest::{Source, QUERY_BYTES};
        use crate::treesitter::{BUILTIN_GRAMMARS, BUILTIN_QUERIES};
        let (registry, errors) = crate::registry::Registry::bundled();
        assert!(errors.is_empty(), "{errors:?}");
        let syntaxes: syntect::parsing::SyntaxSet =
            syntect::dumps::from_uncompressed_data(include_bytes!(concat!(env!("OUT_DIR"), "/syntaxes.packdump")))
                .unwrap();
        let mut ids: Vec<_> = Language::ALL.iter().map(|l| l.registry_id()).collect();
        ids.sort();
        let (builtin, added): (Vec<_>, Vec<_>) =
            registry.languages().iter().partition(|l| Language::from_registry_id(l.id()).is_some());
        assert_eq!(builtin.iter().map(|l| l.id()).collect::<Vec<_>>(), ids);
        // languages added as data alone bring no compiled-in parser
        for language in &added {
            let grammar = language.manifest.tree_sitter.as_ref().map(|t| Source::of(&t.grammar));
            assert!(!matches!(grammar, Some(Source::Builtin(_))), "{}", language.id());
        }
        let servers = default_servers();
        for language in Language::ALL {
            let def = registry.get(language.registry_id()).unwrap();
            let manifest = &def.manifest;
            let name = crate::treesitter::name(language);
            assert_eq!((manifest.name.as_str(), manifest.scope.as_str()), (name, crate::treesitter::scope(language)));
            // the same name means the same language; only the first by id is named
            assert_eq!(manifest.hidden, crate::treesitter::named(name) != Some(language), "{name}");
            let highlighter = crate::highlight::Highlighter::by_name(name);
            assert_eq!(highlighter.syntax_name(), name);
            assert_eq!(manifest.comments.line.as_deref(), highlighter.comment_prefix(), "{name}");
            let block = manifest.comments.block.as_ref().map(|[a, b]| (a.as_str(), b.as_str()));
            assert_eq!(block, highlighter.block_comment(), "{name}");
            let syntax = [name, crate::treesitter::fallback_name(language)]
                .into_iter()
                .find(|n| syntaxes.find_syntax_by_name(n).is_some())
                .map(|n| format!("builtin:{n}"));
            assert_eq!(manifest.syntax, syntax, "{name}");

            let lsp = manifest.lsp.as_ref().unwrap();
            assert_eq!((lsp.language_id.as_str(), manifest.server()), (language.id(), Some(language.server())));
            assert_eq!(ServerLanguage::manifest(def).unwrap(), ServerLanguage::bundled(language));
            let server = &servers[language.server()];
            assert_eq!(lsp.command.as_deref().unwrap_or_default(), server.command, "{name}");
            assert_eq!(lsp.initialization_options.as_ref(), Some(&server.initialization_options), "{name}");

            let ts = manifest.tree_sitter.as_ref().unwrap();
            assert!(matches!(Source::of(&ts.grammar), Source::Builtin(g) if BUILTIN_GRAMMARS.contains(&g)));
            for query in &ts.highlights {
                match Source::of(query) {
                    Source::Builtin(q) => assert!(BUILTIN_QUERIES.contains(&q), "{query}"),
                    Source::File(f) => assert!(!def.read(f, QUERY_BYTES).unwrap().is_empty(), "{f}"),
                }
            }
            for (file, limit) in manifest.files() {
                def.read(file, limit).unwrap();
            }
            assert!(manifest.indent.is_none() && manifest.formatter.is_none(), "{name}");
            assert!(manifest.files.first_line.is_none() && manifest.files.content.is_none(), "{name}");
            assert!(ts.folds.is_none() && ts.indents.is_none() && ts.tags.is_none(), "{name}");

            assert_eq!(
                ServerLanguage::declared(&registry, language.registry_id()),
                Some(ServerLanguage::bundled(language))
            );
            let named = crate::treesitter::named(name).map(ServerLanguage::bundled);
            assert_eq!(ServerLanguage::named(&registry, name), named, "{name}");
            // every claim a manifest makes is one the enum makes
            let files = &manifest.files;
            let claims = files.extensions.iter().map(|e| format!("main.{e}")).chain(files.names.iter().cloned());
            for file in claims {
                assert_eq!(Language::detect(Path::new(&file)), Some(language), "{file}");
            }
        }
        // and every claim the enum makes is the manifest's, through the registry
        for &(file, language) in SAMPLES {
            let path = Path::new(file);
            assert_eq!(Language::detect(path), language, "{file}");
            assert_eq!(registry.detect(Some(path), "").map(|l| l.id()), language.map(Language::registry_id), "{file}");
            assert_eq!(ServerLanguage::detect(&registry, path, ""), language.map(ServerLanguage::bundled), "{file}");
        }
        let mut runtime = runtime_servers(&registry);
        for language in &added {
            if let Some(family) = language.manifest.server().filter(|f| !servers.contains_key(*f)) {
                runtime.remove(family);
            }
        }
        assert_eq!(runtime, servers);
    }

    #[test]
    fn modeline_catalog_ids_keep_each_bundled_protocol_identity() {
        for registry in [crate::registry::Registry::default(), crate::registry::Registry::bundled().0] {
            for language in Language::ALL {
                let declared = ServerLanguage::declared(&registry, language.registry_id()).unwrap();
                assert_eq!(declared.id, language.id());
                assert_eq!(declared.server, language.server());
            }
            assert!(ServerLanguage::declared(&registry, "Plain Text").is_none());
            assert!(ServerLanguage::declared(&registry, "../../bin/sh").is_none());
        }
    }

    #[test]
    fn completion_uses_the_server_range_and_preserves_snippet_source() {
        let rope = Rope::from_str("😀pri");
        let text = Coords::new(&rope, 1);
        let edit = completion_edits(
            text,
            4,
            &json!({"label":"print", "textEdit":{
            "range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},
            "newText":"print"}}),
            Encoding::Utf16,
        )
        .unwrap();
        assert_eq!(edit, vec![(1..4, "print".into())]);
        assert_eq!(
            completion_edits(
                text,
                4,
                &json!({"label":"f", "insertTextFormat":2,
            "insertText":"f($1)"}),
                Encoding::Utf16
            )
            .unwrap(),
            vec![(1..4, "f($1)".into())]
        );
    }

    #[test]
    fn semantic_tokens_validate_unicode_and_legend_indices() {
        let rope = Rope::from_str("😀foo\nbar");
        let text = Coords::new(&rope, 1);
        let legend = json!({"tokenTypes":["function"],"tokenModifiers":[]});
        let spans = semantic_tokens(text, &json!({"data":[0,2,3,0,0,1,0,3,0,0]}), &legend, Encoding::Utf16).unwrap();
        assert_eq!(spans.iter().map(|s| s.range.clone()).collect::<Vec<_>>(), vec![1..4, 5..8]);
        assert!(semantic_tokens(text, &json!({"data":[0,1,1,0,0]}), &legend, Encoding::Utf16).is_err());
        assert!(semantic_tokens(text, &json!({"data":[0,2,3,9,0]}), &legend, Encoding::Utf16).is_err());
    }

    #[test]
    fn hover_and_location_links_keep_real_contents() {
        assert_eq!(
            hover_text(&json!({"contents":[{"language":"rust","value":"fn f()"},
            {"kind":"markdown","value":"docs"}]})),
            "fn f()\n\ndocs"
        );
        let links = locations(&json!([{"targetUri":"file:///tmp/a.rs","targetSelectionRange":{
            "start":{"line":2,"character":1},"end":{"line":2,"character":2}}}]))
        .unwrap();
        assert_eq!(links[0].0, "file:///tmp/a.rs");
        assert_eq!(links[0].1.start, Position::new(2, 1));
    }

    #[test]
    fn location_ranges_reject_invalid_protocol_coordinates_before_display() {
        for coordinate in [json!(u64::MAX), json!(-1), json!("1"), Value::Null] {
            for endpoint in ["start", "end"] {
                for component in ["line", "character"] {
                    let mut range = json!({"start":{"line":0,"character":0},"end":{"line":0,"character":1}});
                    range[endpoint][component] = coordinate.clone();
                    assert!(locations(&json!({"uri":"file:///tmp/a.rs","range":range})).is_err());
                }
            }
        }
    }
}
