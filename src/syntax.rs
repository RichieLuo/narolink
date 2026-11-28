//! Fast, self-contained syntax tokenizer for desktop and web code views.
use std::{ops::Range, path::Path};

pub(crate) const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TokenKind {
    Plain,
    Comment,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Constant,
    Property,
    Tag,
    Operator,
    Escape,
    Added,
    Removed,
}

impl TokenKind {
    pub(crate) fn css_class(self) -> Option<&'static str> {
        match self {
            Self::Plain => None,
            Self::Comment => Some("tok-comment"),
            Self::Keyword => Some("tok-keyword"),
            Self::Type => Some("tok-type"),
            Self::Function => Some("tok-fn"),
            Self::String => Some("tok-string"),
            Self::Number => Some("tok-number"),
            Self::Constant => Some("tok-constant"),
            Self::Property => Some("tok-property"),
            Self::Tag => Some("tok-tag"),
            Self::Operator => Some("tok-operator"),
            Self::Escape => Some("tok-escape"),
            Self::Added => Some("tok-added"),
            Self::Removed => Some("tok-removed"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LanguageKind {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    Cpp,
    Java,
    Kotlin,
    Swift,
    CSharp,
    Ruby,
    Php,
    Lua,
    Zig,
    Dart,
    Shell,
    Sql,
    Html,
    Css,
    Json,
    Yaml,
    Toml,
    Ini,
    Markdown,
    Dockerfile,
    Makefile,
    Proto,
    Graphql,
    Diff,
}

pub(crate) fn detect_syntax_from_path(path: &Path) -> String {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let lower = name.to_ascii_lowercase();
        if lower == "dockerfile" || lower.starts_with("dockerfile.") {
            return "dockerfile".into();
        }
        if matches!(lower.as_str(), "makefile" | "gnumakefile" | "justfile") {
            return "makefile".into();
        }
        if lower == "cmakelists.txt" {
            return "cmake".into();
        }
        if lower == "cargo.lock" {
            return "toml".into();
        }
        if matches!(
            lower.as_str(),
            "gemfile" | "rakefile" | "podfile" | "fastfile"
        ) {
            return "rb".into();
        }
        if matches!(
            lower.as_str(),
            ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" | ".zprofile"
        ) {
            return "sh".into();
        }
        if lower == ".env" || lower.starts_with(".env.") {
            return "env".into();
        }
        if matches!(
            lower.as_str(),
            ".gitignore" | ".dockerignore" | ".editorconfig"
        ) {
            return "ini".into();
        }
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub(crate) fn resolve_language(lang: &str) -> Option<LanguageKind> {
    let token = lang.trim().split_whitespace().next().unwrap_or_default();
    let lower = token.to_ascii_lowercase();
    let kind = match lower.as_str() {
        "rs" | "rust" => LanguageKind::Rust,
        "js" | "jsx" | "mjs" | "cjs" | "javascript" => LanguageKind::JavaScript,
        "ts" | "tsx" | "mts" | "cts" | "typescript" => LanguageKind::TypeScript,
        "py" | "pyi" | "python" => LanguageKind::Python,
        "go" | "golang" => LanguageKind::Go,
        "c" | "h" | "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "m" | "mm" | "objc" => {
            LanguageKind::Cpp
        }
        "java" | "scala" | "groovy" => LanguageKind::Java,
        "kt" | "kts" | "kotlin" => LanguageKind::Kotlin,
        "swift" => LanguageKind::Swift,
        "cs" | "csharp" | "c#" => LanguageKind::CSharp,
        "rb" | "ruby" => LanguageKind::Ruby,
        "php" => LanguageKind::Php,
        "lua" => LanguageKind::Lua,
        "zig" => LanguageKind::Zig,
        "dart" => LanguageKind::Dart,
        "sh" | "bash" | "zsh" | "shell" | "fish" | "ps1" | "powershell" | "bat" | "cmd"
        | "cmake" => LanguageKind::Shell,
        "sql" => LanguageKind::Sql,
        "html" | "htm" | "xml" | "svg" | "plist" | "vue" | "svelte" | "xhtml" => LanguageKind::Html,
        "css" | "scss" | "sass" | "less" => LanguageKind::Css,
        "json" | "jsonc" | "json5" | "webmanifest" => LanguageKind::Json,
        "yml" | "yaml" => LanguageKind::Yaml,
        "toml" => LanguageKind::Toml,
        "ini" | "cfg" | "conf" | "env" | "properties" | "dotenv" => LanguageKind::Ini,
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" => LanguageKind::Markdown,
        "dockerfile" | "docker" => LanguageKind::Dockerfile,
        "makefile" | "make" | "mk" | "just" => LanguageKind::Makefile,
        "proto" | "protobuf" => LanguageKind::Proto,
        "graphql" | "gql" => LanguageKind::Graphql,
        "diff" | "patch" => LanguageKind::Diff,
        _ => return None,
    };
    Some(kind)
}

pub(crate) fn tokenize(source: &str, lang: &str) -> Vec<Token> {
    if source.is_empty() {
        return Vec::new();
    }
    let Some(kind) = resolve_language(lang) else {
        return vec![Token {
            range: 0..source.len(),
            kind: TokenKind::Plain,
        }];
    };
    let mut out = TokenSink::new();
    tokenize_into(source, kind, &mut out);
    out.finish(source.len())
}

pub(crate) fn highlight_html(source: &str, lang: &str) -> Option<String> {
    if source.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    resolve_language(lang)?;
    let tokens = tokenize(source, lang);
    let mut html = String::with_capacity(source.len() + tokens.len() * 24);
    for token in tokens {
        let slice = &source[token.range];
        if let Some(class) = token.kind.css_class() {
            html.push_str("<span class=\"");
            html.push_str(class);
            html.push_str("\">");
            push_escaped_html(&mut html, slice);
            html.push_str("</span>");
        } else {
            push_escaped_html(&mut html, slice);
        }
    }
    Some(html)
}

fn push_escaped_html(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

struct TokenSink {
    tokens: Vec<Token>,
    cursor: usize,
}

impl TokenSink {
    fn new() -> Self {
        Self {
            tokens: Vec::new(),
            cursor: 0,
        }
    }

    fn push(&mut self, range: Range<usize>, kind: TokenKind) {
        if range.start >= range.end {
            return;
        }
        if range.start > self.cursor {
            self.push_raw(self.cursor..range.start, TokenKind::Plain);
        }
        let start = range.start.max(self.cursor);
        if start < range.end {
            self.push_raw(start..range.end, kind);
            self.cursor = range.end;
        }
    }

    fn push_raw(&mut self, range: Range<usize>, kind: TokenKind) {
        if let Some(last) = self.tokens.last_mut()
            && last.kind == kind
            && last.range.end == range.start
        {
            last.range.end = range.end;
            return;
        }
        self.tokens.push(Token { range, kind });
    }

    fn finish(mut self, len: usize) -> Vec<Token> {
        if self.cursor < len {
            self.push_raw(self.cursor..len, TokenKind::Plain);
        }
        self.tokens
    }
}

fn tokenize_into(source: &str, kind: LanguageKind, out: &mut TokenSink) {
    match kind {
        LanguageKind::Json => tokenize_json(source, 0, out),
        LanguageKind::Yaml => tokenize_yaml(source, 0, out),
        LanguageKind::Toml => tokenize_toml_ini(source, 0, true, out),
        LanguageKind::Ini => tokenize_toml_ini(source, 0, false, out),
        LanguageKind::Html => tokenize_html(source, 0, out),
        LanguageKind::Css => tokenize_css(source, 0, out),
        LanguageKind::Markdown => tokenize_markdown(source, 0, out),
        LanguageKind::Diff => tokenize_diff(source, 0, out),
        LanguageKind::Makefile => tokenize_makefile(source, 0, out),
        _ => tokenize_code(source, 0, lang_spec(kind), out),
    }
}

struct LangSpec {
    keywords: &'static [&'static str],
    constants: &'static [&'static str],
    types: &'static [&'static str],
    slash_comments: bool,
    hash_comments: bool,
    dash_comments: bool,
    block_comment: bool,
    nested_block_comment: bool,
    single_quote_string: bool,
    backtick_string: bool,
    triple_quote: bool,
    rust_strings_and_lifetimes: bool,
    shell_vars: bool,
    hash_directives: bool,
    at_decorators: bool,
    macro_bang: bool,
    jsx_tags: bool,
    case_insensitive: bool,
}

fn lang_spec(kind: LanguageKind) -> LangSpec {
    match kind {
        LanguageKind::Rust => LangSpec {
            keywords: &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
                "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
                "move", "mut", "pub", "ref", "return", "static", "struct", "super", "trait",
                "type", "unsafe", "use", "where", "while", "yield",
            ],
            constants: &["true", "false", "self", "Self", "Some", "None", "Ok", "Err"],
            types: &[
                "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16",
                "i32", "i64", "i128", "isize", "f32", "f64", "String", "Vec", "Option", "Result",
                "Box", "Rc", "Arc", "Cell", "RefCell", "HashMap", "HashSet", "BTreeMap",
                "BTreeSet", "Path", "PathBuf", "Cow",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: true,
            single_quote_string: false,
            backtick_string: false,
            triple_quote: false,
            rust_strings_and_lifetimes: true,
            shell_vars: false,
            hash_directives: true,
            at_decorators: false,
            macro_bang: true,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::JavaScript | LanguageKind::TypeScript => LangSpec {
            keywords: &[
                "abstract",
                "as",
                "asserts",
                "async",
                "await",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "debugger",
                "declare",
                "default",
                "delete",
                "do",
                "else",
                "enum",
                "export",
                "extends",
                "finally",
                "for",
                "from",
                "function",
                "get",
                "if",
                "implements",
                "import",
                "in",
                "infer",
                "instanceof",
                "interface",
                "is",
                "keyof",
                "let",
                "module",
                "namespace",
                "new",
                "of",
                "override",
                "package",
                "private",
                "protected",
                "public",
                "readonly",
                "return",
                "satisfies",
                "set",
                "static",
                "super",
                "switch",
                "throw",
                "try",
                "type",
                "typeof",
                "using",
                "var",
                "void",
                "while",
                "with",
                "yield",
            ],
            constants: &[
                "true",
                "false",
                "null",
                "undefined",
                "this",
                "NaN",
                "Infinity",
                "globalThis",
            ],
            types: &[
                "any", "bigint", "boolean", "never", "number", "object", "string", "symbol",
                "unknown", "Array", "Promise", "Map", "Set", "Record", "Partial", "Required",
                "Readonly", "Pick", "Omit", "Date", "Error", "RegExp", "URL", "Response",
                "Request",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: true,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: true,
            case_insensitive: false,
        },
        LanguageKind::Python => LangSpec {
            keywords: &[
                "and", "as", "assert", "async", "await", "break", "case", "class", "continue",
                "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
                "import", "in", "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise",
                "return", "try", "type", "while", "with", "yield",
            ],
            constants: &["True", "False", "None", "self", "cls", "Ellipsis"],
            types: &[
                "int",
                "float",
                "str",
                "bool",
                "bytes",
                "bytearray",
                "list",
                "dict",
                "set",
                "frozenset",
                "tuple",
                "object",
                "Any",
                "Optional",
                "Union",
                "Callable",
                "Iterable",
                "Sequence",
                "Mapping",
                "Path",
                "Exception",
            ],
            slash_comments: false,
            hash_comments: true,
            dash_comments: false,
            block_comment: false,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: true,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Go => LangSpec {
            keywords: &[
                "break",
                "case",
                "chan",
                "const",
                "continue",
                "default",
                "defer",
                "else",
                "fallthrough",
                "for",
                "func",
                "go",
                "goto",
                "if",
                "import",
                "interface",
                "map",
                "package",
                "range",
                "return",
                "select",
                "struct",
                "switch",
                "type",
                "var",
            ],
            constants: &["true", "false", "nil", "iota"],
            types: &[
                "any",
                "bool",
                "byte",
                "comparable",
                "complex64",
                "complex128",
                "error",
                "float32",
                "float64",
                "int",
                "int8",
                "int16",
                "int32",
                "int64",
                "rune",
                "string",
                "uint",
                "uint8",
                "uint16",
                "uint32",
                "uint64",
                "uintptr",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: true,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: false,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Cpp => LangSpec {
            keywords: &[
                "alignas",
                "alignof",
                "auto",
                "break",
                "case",
                "catch",
                "class",
                "co_await",
                "co_return",
                "co_yield",
                "concept",
                "const",
                "consteval",
                "constexpr",
                "constinit",
                "continue",
                "decltype",
                "default",
                "delete",
                "do",
                "else",
                "enum",
                "explicit",
                "export",
                "extern",
                "final",
                "for",
                "friend",
                "goto",
                "if",
                "inline",
                "mutable",
                "namespace",
                "new",
                "noexcept",
                "operator",
                "override",
                "private",
                "protected",
                "public",
                "register",
                "requires",
                "return",
                "sizeof",
                "static",
                "static_assert",
                "struct",
                "switch",
                "template",
                "throw",
                "try",
                "typedef",
                "typename",
                "union",
                "using",
                "virtual",
                "volatile",
                "while",
            ],
            constants: &["true", "false", "nullptr", "NULL", "this"],
            types: &[
                "void", "bool", "char", "short", "int", "long", "float", "double", "signed",
                "unsigned", "size_t", "ssize_t", "int8_t", "int16_t", "int32_t", "int64_t",
                "uint8_t", "uint16_t", "uint32_t", "uint64_t", "string", "vector",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: true,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Java | LanguageKind::CSharp => LangSpec {
            keywords: &[
                "abstract",
                "as",
                "async",
                "await",
                "base",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "default",
                "delegate",
                "do",
                "else",
                "enum",
                "event",
                "extends",
                "final",
                "finally",
                "for",
                "foreach",
                "get",
                "if",
                "implements",
                "import",
                "in",
                "instanceof",
                "interface",
                "internal",
                "is",
                "lock",
                "namespace",
                "new",
                "out",
                "override",
                "package",
                "params",
                "partial",
                "private",
                "protected",
                "public",
                "readonly",
                "record",
                "ref",
                "return",
                "sealed",
                "set",
                "static",
                "struct",
                "super",
                "switch",
                "synchronized",
                "throw",
                "throws",
                "try",
                "typeof",
                "using",
                "var",
                "virtual",
                "volatile",
                "when",
                "where",
                "while",
                "yield",
            ],
            constants: &["true", "false", "null", "this"],
            types: &[
                "void", "bool", "boolean", "byte", "sbyte", "char", "short", "ushort", "int",
                "uint", "long", "ulong", "float", "double", "decimal", "string", "object",
                "String", "Object", "List", "Map", "Set", "Task",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: true,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: true,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Kotlin => LangSpec {
            keywords: &[
                "abstract",
                "actual",
                "annotation",
                "as",
                "break",
                "by",
                "catch",
                "class",
                "companion",
                "const",
                "constructor",
                "continue",
                "crossinline",
                "data",
                "do",
                "else",
                "enum",
                "expect",
                "external",
                "final",
                "finally",
                "for",
                "fun",
                "get",
                "if",
                "import",
                "in",
                "infix",
                "init",
                "inline",
                "inner",
                "interface",
                "internal",
                "is",
                "lateinit",
                "noinline",
                "object",
                "open",
                "operator",
                "out",
                "override",
                "package",
                "private",
                "protected",
                "public",
                "reified",
                "return",
                "sealed",
                "set",
                "super",
                "suspend",
                "tailrec",
                "throw",
                "try",
                "typealias",
                "val",
                "var",
                "vararg",
                "when",
                "where",
                "while",
            ],
            constants: &["true", "false", "null", "this"],
            types: &[
                "Unit", "Nothing", "Any", "Boolean", "Byte", "Short", "Int", "Long", "Float",
                "Double", "Char", "String", "List", "Map", "Set",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: true,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: true,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Swift => LangSpec {
            keywords: &[
                "actor",
                "any",
                "as",
                "associatedtype",
                "async",
                "await",
                "break",
                "case",
                "catch",
                "class",
                "continue",
                "default",
                "defer",
                "deinit",
                "do",
                "else",
                "enum",
                "extension",
                "fallthrough",
                "fileprivate",
                "final",
                "for",
                "func",
                "get",
                "guard",
                "if",
                "import",
                "in",
                "indirect",
                "infix",
                "init",
                "inout",
                "internal",
                "is",
                "lazy",
                "let",
                "mutating",
                "nonisolated",
                "nonmutating",
                "open",
                "operator",
                "optional",
                "override",
                "prefix",
                "private",
                "protocol",
                "public",
                "repeat",
                "required",
                "rethrows",
                "return",
                "set",
                "some",
                "static",
                "struct",
                "subscript",
                "super",
                "switch",
                "throw",
                "throws",
                "try",
                "typealias",
                "unowned",
                "var",
                "weak",
                "where",
                "while",
            ],
            constants: &["true", "false", "nil", "self", "Self"],
            types: &[
                "Void",
                "Bool",
                "Int",
                "UInt",
                "Float",
                "Double",
                "String",
                "Character",
                "Array",
                "Dictionary",
                "Set",
                "Optional",
                "Result",
                "URL",
                "Data",
                "Error",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: true,
            single_quote_string: false,
            backtick_string: false,
            triple_quote: true,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: true,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Ruby => LangSpec {
            keywords: &[
                "alias",
                "and",
                "begin",
                "break",
                "case",
                "class",
                "def",
                "defined?",
                "do",
                "else",
                "elsif",
                "end",
                "ensure",
                "for",
                "if",
                "in",
                "module",
                "next",
                "not",
                "or",
                "redo",
                "require",
                "require_relative",
                "rescue",
                "retry",
                "return",
                "super",
                "then",
                "undef",
                "unless",
                "until",
                "when",
                "while",
                "yield",
            ],
            constants: &["true", "false", "nil", "self"],
            types: &[
                "String", "Integer", "Float", "Array", "Hash", "Symbol", "Object",
            ],
            slash_comments: false,
            hash_comments: true,
            dash_comments: false,
            block_comment: false,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: true,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Php => LangSpec {
            keywords: &[
                "abstract",
                "and",
                "array",
                "as",
                "break",
                "callable",
                "case",
                "catch",
                "class",
                "clone",
                "const",
                "continue",
                "declare",
                "default",
                "do",
                "echo",
                "else",
                "elseif",
                "empty",
                "enum",
                "extends",
                "final",
                "finally",
                "fn",
                "for",
                "foreach",
                "function",
                "global",
                "goto",
                "if",
                "implements",
                "include",
                "include_once",
                "instanceof",
                "interface",
                "isset",
                "list",
                "match",
                "namespace",
                "new",
                "or",
                "print",
                "private",
                "protected",
                "public",
                "readonly",
                "require",
                "require_once",
                "return",
                "static",
                "switch",
                "throw",
                "trait",
                "try",
                "unset",
                "use",
                "var",
                "while",
                "xor",
                "yield",
            ],
            constants: &["true", "false", "null", "self", "parent"],
            types: &[
                "int", "float", "string", "bool", "void", "mixed", "never", "object",
            ],
            slash_comments: true,
            hash_comments: true,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: true,
            hash_directives: false,
            at_decorators: false,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Lua => LangSpec {
            keywords: &[
                "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if",
                "in", "local", "not", "or", "repeat", "return", "then", "until", "while",
            ],
            constants: &["true", "false", "nil", "self"],
            types: &[],
            slash_comments: false,
            hash_comments: false,
            dash_comments: true,
            block_comment: false,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: false,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Zig => LangSpec {
            keywords: &[
                "addrspace",
                "align",
                "allowzero",
                "and",
                "anyframe",
                "asm",
                "async",
                "await",
                "break",
                "callconv",
                "catch",
                "comptime",
                "const",
                "continue",
                "defer",
                "else",
                "enum",
                "errdefer",
                "error",
                "export",
                "extern",
                "fn",
                "for",
                "if",
                "inline",
                "linksection",
                "noalias",
                "noinline",
                "nosuspend",
                "opaque",
                "or",
                "orelse",
                "packed",
                "pub",
                "resume",
                "return",
                "struct",
                "suspend",
                "switch",
                "test",
                "threadlocal",
                "try",
                "union",
                "unreachable",
                "var",
                "volatile",
                "while",
            ],
            constants: &["true", "false", "null", "undefined"],
            types: &[
                "void", "bool", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32",
                "i64", "i128", "isize", "f16", "f32", "f64", "anyerror", "anytype", "type",
                "noreturn",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: false,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Dart => LangSpec {
            keywords: &[
                "abstract",
                "as",
                "assert",
                "async",
                "await",
                "base",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "covariant",
                "default",
                "deferred",
                "do",
                "else",
                "enum",
                "export",
                "extends",
                "extension",
                "external",
                "factory",
                "final",
                "finally",
                "for",
                "get",
                "hide",
                "if",
                "implements",
                "import",
                "in",
                "interface",
                "is",
                "late",
                "library",
                "mixin",
                "new",
                "on",
                "operator",
                "part",
                "required",
                "rethrow",
                "return",
                "sealed",
                "set",
                "show",
                "static",
                "super",
                "switch",
                "sync",
                "throw",
                "try",
                "typedef",
                "var",
                "when",
                "while",
                "with",
                "yield",
            ],
            constants: &["true", "false", "null", "this"],
            types: &[
                "void", "bool", "int", "double", "num", "String", "List", "Map", "Set", "Future",
                "Stream", "Object", "dynamic",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: true,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Shell => LangSpec {
            keywords: &[
                "alias", "bg", "break", "case", "cd", "command", "continue", "declare", "do",
                "done", "echo", "elif", "else", "esac", "eval", "exec", "exit", "export", "fg",
                "fi", "for", "function", "if", "in", "local", "printf", "read", "readonly",
                "return", "select", "set", "shift", "source", "then", "time", "trap", "typeset",
                "unset", "until", "wait", "while",
            ],
            constants: &["true", "false"],
            types: &[],
            slash_comments: false,
            hash_comments: true,
            dash_comments: false,
            block_comment: false,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: true,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: true,
            hash_directives: false,
            at_decorators: false,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Sql => LangSpec {
            keywords: &[
                "add",
                "all",
                "alter",
                "and",
                "as",
                "asc",
                "begin",
                "between",
                "by",
                "case",
                "check",
                "column",
                "commit",
                "constraint",
                "create",
                "cross",
                "database",
                "default",
                "delete",
                "desc",
                "distinct",
                "drop",
                "else",
                "end",
                "exists",
                "explain",
                "foreign",
                "from",
                "full",
                "group",
                "having",
                "if",
                "in",
                "index",
                "inner",
                "insert",
                "into",
                "is",
                "join",
                "key",
                "left",
                "like",
                "limit",
                "not",
                "offset",
                "on",
                "or",
                "order",
                "outer",
                "pragma",
                "primary",
                "references",
                "replace",
                "returning",
                "right",
                "rollback",
                "select",
                "set",
                "table",
                "then",
                "transaction",
                "union",
                "unique",
                "update",
                "using",
                "values",
                "view",
                "when",
                "where",
                "with",
            ],
            constants: &["true", "false", "null"],
            types: &[
                "int",
                "integer",
                "bigint",
                "smallint",
                "real",
                "float",
                "double",
                "numeric",
                "decimal",
                "text",
                "varchar",
                "char",
                "blob",
                "boolean",
                "bool",
                "date",
                "time",
                "timestamp",
                "json",
                "jsonb",
                "uuid",
            ],
            slash_comments: false,
            hash_comments: false,
            dash_comments: true,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: true,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: false,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: true,
        },
        LanguageKind::Dockerfile => LangSpec {
            keywords: &[
                "add",
                "arg",
                "as",
                "cmd",
                "copy",
                "entrypoint",
                "env",
                "expose",
                "from",
                "healthcheck",
                "label",
                "maintainer",
                "onbuild",
                "run",
                "shell",
                "stopsignal",
                "user",
                "volume",
                "workdir",
            ],
            constants: &["true", "false"],
            types: &[],
            slash_comments: false,
            hash_comments: true,
            dash_comments: false,
            block_comment: false,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: true,
            hash_directives: false,
            at_decorators: false,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: true,
        },
        LanguageKind::Proto => LangSpec {
            keywords: &[
                "edition",
                "enum",
                "extend",
                "extensions",
                "import",
                "map",
                "message",
                "oneof",
                "option",
                "optional",
                "package",
                "public",
                "repeated",
                "required",
                "reserved",
                "returns",
                "rpc",
                "service",
                "stream",
                "syntax",
                "to",
            ],
            constants: &["true", "false"],
            types: &[
                "double", "float", "int32", "int64", "uint32", "uint64", "sint32", "sint64",
                "fixed32", "fixed64", "sfixed32", "sfixed64", "bool", "string", "bytes",
            ],
            slash_comments: true,
            hash_comments: false,
            dash_comments: false,
            block_comment: true,
            nested_block_comment: false,
            single_quote_string: true,
            backtick_string: false,
            triple_quote: false,
            rust_strings_and_lifetimes: false,
            shell_vars: false,
            hash_directives: false,
            at_decorators: false,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        LanguageKind::Graphql => LangSpec {
            keywords: &[
                "directive",
                "enum",
                "extend",
                "fragment",
                "implements",
                "input",
                "interface",
                "mutation",
                "on",
                "query",
                "repeatable",
                "scalar",
                "schema",
                "subscription",
                "type",
                "union",
            ],
            constants: &["true", "false", "null"],
            types: &["Int", "Float", "String", "Boolean", "ID"],
            slash_comments: false,
            hash_comments: true,
            dash_comments: false,
            block_comment: false,
            nested_block_comment: false,
            single_quote_string: false,
            backtick_string: false,
            triple_quote: true,
            rust_strings_and_lifetimes: false,
            shell_vars: true,
            hash_directives: false,
            at_decorators: true,
            macro_bang: false,
            jsx_tags: false,
            case_insensitive: false,
        },
        _ => unreachable!(),
    }
}

fn tokenize_code(source: &str, base: usize, spec: LangSpec, out: &mut TokenSink) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Whitespace
        if b.is_ascii_whitespace() {
            let start = i;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Plain);
            continue;
        }

        // Line comments
        if (spec.slash_comments && bytes[i..].starts_with(b"//"))
            || (spec.hash_comments && b == b'#')
            || (spec.dash_comments && bytes[i..].starts_with(b"--"))
        {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Comment);
            continue;
        }

        // Block comments /* ... */
        if spec.block_comment && bytes[i..].starts_with(b"/*") {
            let start = i;
            i += 2;
            let mut depth = 1usize;
            while i < len && depth > 0 {
                if spec.nested_block_comment && bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(base + start..base + i, TokenKind::Comment);
            continue;
        }

        // Rust attributes (#[...] / #![...]) and C/Swift/C# #directives
        if spec.hash_directives && b == b'#' {
            if bytes[i..].starts_with(b"#[") || bytes[i..].starts_with(b"#![") {
                let start = i;
                i += if bytes[i + 1] == b'!' { 3 } else { 2 };
                let mut depth = 1usize;
                while i < len && depth > 0 {
                    match bytes[i] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'"' => {
                            i += 1;
                            while i < len && bytes[i] != b'"' {
                                if bytes[i] == b'\\' && i + 1 < len {
                                    i += 2;
                                } else {
                                    i += 1;
                                }
                            }
                        }
                        _ => {}
                    }
                    if i < len {
                        i += 1;
                    }
                }
                out.push(base + start..base + i, TokenKind::Property);
                continue;
            }
            let start = i;
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() && bytes[i] != b'\n' {
                i += 1;
            }
            let word_start = i;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            if i > word_start {
                out.push(base + start..base + i, TokenKind::Property);
                continue;
            }
            out.push(base + start..base + start + 1, TokenKind::Operator);
            i = start + 1;
            continue;
        }

        // @decorators / @annotations / @builtins
        if spec.at_decorators && b == b'@' && i + 1 < len && is_ident_start(bytes[i + 1]) {
            let start = i;
            i += 1;
            while i < len && (is_ident_continue(bytes[i]) || bytes[i] == b'.') {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Property);
            continue;
        }

        // Shell / PHP / GraphQL $variables
        if spec.shell_vars && b == b'$' {
            let start = i;
            if i + 1 < len && bytes[i + 1] == b'{' {
                i += 2;
                while i < len && bytes[i] != b'}' && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < len && bytes[i] == b'}' {
                    i += 1;
                }
                out.push(base + start..base + i, TokenKind::Type);
                continue;
            }
            if i + 1 < len
                && (is_ident_start(bytes[i + 1])
                    || bytes[i + 1].is_ascii_digit()
                    || matches!(bytes[i + 1], b'?' | b'#' | b'@' | b'*' | b'$' | b'!'))
            {
                i += 1;
                if is_ident_start(bytes[i]) {
                    while i < len && is_ident_continue(bytes[i]) {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
                out.push(base + start..base + i, TokenKind::Type);
                continue;
            }
        }

        // Rust raw/byte strings: r"...", r#"..."#, b"...", br#"..."#, c"..."
        if spec.rust_strings_and_lifetimes
            && let Some(end) = try_scan_rust_raw_string(source, i)
        {
            out.push(base + i..base + end, TokenKind::String);
            i = end;
            continue;
        }

        // Python string prefixes r"...", f"...", b"...", u"..."
        if spec.triple_quote
            && matches!(b, b'r' | b'R' | b'f' | b'F' | b'b' | b'B' | b'u' | b'U')
            && i + 1 < len
            && matches!(bytes[i + 1], b'"' | b'\'')
        {
            let quote = bytes[i + 1];
            let end = scan_string(source, i + 1, quote, spec.triple_quote, base, Some(i), out);
            i = end;
            continue;
        }

        // Double-quoted strings
        if b == b'"' {
            i = scan_string(source, i, b'"', spec.triple_quote, base, None, out);
            continue;
        }

        // Single-quoted strings or Rust char literals / lifetimes
        if b == b'\'' {
            if spec.rust_strings_and_lifetimes {
                if let Some(end) = try_scan_rust_char(bytes, i) {
                    emit_string_with_escapes(source, i, end, base, out);
                    i = end;
                    continue;
                }
                // Rust lifetime like 'a or 'static
                if i + 1 < len && is_ident_start(bytes[i + 1]) {
                    let start = i;
                    i += 1;
                    while i < len && is_ident_continue(bytes[i]) {
                        i += 1;
                    }
                    out.push(base + start..base + i, TokenKind::Type);
                    continue;
                }
            } else if spec.single_quote_string {
                i = scan_string(source, i, b'\'', spec.triple_quote, base, None, out);
                continue;
            }
        }

        // Backtick strings
        if spec.backtick_string && b == b'`' {
            i = scan_backtick_string(source, i, base, out);
            continue;
        }

        // JSX opening/closing tags like <Component ...> or </div>
        if spec.jsx_tags
            && b == b'<'
            && let Some(end) = try_scan_jsx_tag_name(bytes, i)
        {
            let tag_start = if bytes[i + 1] == b'/' { i + 2 } else { i + 1 };
            out.push(base + i..base + tag_start, TokenKind::Operator);
            out.push(base + tag_start..base + end, TokenKind::Tag);
            i = end;
            continue;
        }

        // Numbers
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            i = scan_number(bytes, i);
            out.push(base + start..base + i, TokenKind::Number);
            continue;
        }

        // Identifiers, keywords, types, functions, macros
        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &source[start..i];

            // Rust macro invocation: foo!
            if spec.macro_bang && i < len && bytes[i] == b'!' && bytes.get(i + 1) != Some(&b'=') {
                i += 1;
                out.push(base + start..base + i, TokenKind::Function);
                continue;
            }

            let kind = classify_identifier(word, bytes, i, &spec);
            out.push(base + start..base + i, kind);
            continue;
        }

        // Operators and punctuation
        if b.is_ascii_punctuation() {
            let start = i;
            i += 1;
            out.push(base + start..base + i, TokenKind::Operator);
            continue;
        }

        // Non-ASCII character (e.g., CJK identifier or symbol)
        let ch_len = source[i..].chars().next().map_or(1, char::len_utf8);
        out.push(base + i..base + i + ch_len, TokenKind::Plain);
        i += ch_len;
    }
}

fn classify_identifier(word: &str, bytes: &[u8], end: usize, spec: &LangSpec) -> TokenKind {
    let matches_list = |list: &[&str]| {
        if spec.case_insensitive {
            list.iter().any(|item| item.eq_ignore_ascii_case(word))
        } else {
            list.contains(&word)
        }
    };

    if matches_list(spec.keywords) {
        return TokenKind::Keyword;
    }
    if matches_list(spec.constants) {
        return TokenKind::Constant;
    }
    if matches_list(spec.types) {
        return TokenKind::Type;
    }

    // Look ahead past optional horizontal whitespace for function call `foo(`
    let mut look = end;
    while look < bytes.len() && matches!(bytes[look], b' ' | b'\t') {
        look += 1;
    }
    if look < bytes.len() && bytes[look] == b'(' {
        return TokenKind::Function;
    }

    // SCREAMING_SNAKE_CASE constant (at least 2 chars, uppercase/digits/underscores, contains letter)
    if word.len() >= 2
        && word.bytes().any(|b| b.is_ascii_uppercase())
        && word
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return TokenKind::Constant;
    }

    // PascalCase type name (starts with uppercase, has at least one lowercase letter)
    if word.as_bytes()[0].is_ascii_uppercase() && word.bytes().any(|b| b.is_ascii_lowercase()) {
        return TokenKind::Type;
    }

    TokenKind::Plain
}

fn try_scan_rust_raw_string(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut i = start;
    if bytes.get(i) == Some(&b'b') || bytes.get(i) == Some(&b'c') {
        if bytes.get(i + 1) == Some(&b'"') {
            return Some(scan_plain_quoted_end(bytes, i + 1, b'"'));
        }
        if bytes.get(i + 1) == Some(&b'r') {
            i += 1;
        } else {
            return None;
        }
    }
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    i += 1;
    let mut hashes = 0usize;
    while bytes.get(i) == Some(&b'#') {
        hashes += 1;
        i += 1;
    }
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut matched = 0;
            while matched < hashes && bytes.get(i + 1 + matched) == Some(&b'#') {
                matched += 1;
            }
            if matched == hashes {
                return Some(i + 1 + hashes);
            }
        }
        i += 1;
    }
    Some(bytes.len())
}

fn try_scan_rust_char(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'\'') || start + 2 >= bytes.len() {
        return None;
    }
    if bytes[start + 1] == b'\\' {
        let mut i = start + 2;
        while i < bytes.len() && i < start + 14 {
            if bytes[i] == b'\'' {
                return Some(i + 1);
            }
            if bytes[i] == b'\n' {
                return None;
            }
            i += 1;
        }
        return None;
    }
    // Single UTF-8 character followed by closing quote
    let rest = std::str::from_utf8(&bytes[start + 1..]).ok()?;
    let ch = rest.chars().next()?;
    if ch == '\'' || ch == '\n' {
        return None;
    }
    let close = start + 1 + ch.len_utf8();
    if bytes.get(close) == Some(&b'\'') {
        Some(close + 1)
    } else {
        None
    }
}

fn try_scan_jsx_tag_name(bytes: &[u8], start: usize) -> Option<usize> {
    // Only match when preceded by start-of-file, whitespace, or expression delimiters
    if start > 0 {
        let mut prev = start;
        while prev > 0 && matches!(bytes[prev - 1], b' ' | b'\t') {
            prev -= 1;
        }
        if prev > 0 {
            let pb = bytes[prev - 1];
            if is_ident_continue(pb) || matches!(pb, b')' | b']' | b'"' | b'\'') {
                return None;
            }
        }
    }
    let mut i = start + 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    if i >= bytes.len() || !bytes[i].is_ascii_alphabetic() {
        return None;
    }
    let name_start = i;
    while i < bytes.len() && (is_ident_continue(bytes[i]) || matches!(bytes[i], b'-' | b'.')) {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    // Must be followed by whitespace, '>', '/', or newline (not generic `<T, U>` unless closing)
    match bytes.get(i).copied() {
        Some(b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r') => Some(i),
        _ => None,
    }
}

fn scan_plain_quoted_end(bytes: &[u8], quote_pos: usize, quote: u8) -> usize {
    let mut i = quote_pos + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else if bytes[i] == quote {
            return i + 1;
        } else if bytes[i] == b'\n' {
            return i;
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn scan_string(
    source: &str,
    quote_pos: usize,
    quote: u8,
    allow_triple: bool,
    base: usize,
    prefix_start: Option<usize>,
    out: &mut TokenSink,
) -> usize {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let start = prefix_start.unwrap_or(quote_pos);
    let triple = allow_triple
        && quote_pos + 2 < len
        && bytes[quote_pos + 1] == quote
        && bytes[quote_pos + 2] == quote;
    let mut i = quote_pos + if triple { 3 } else { 1 };
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 2;
        } else if triple {
            if i + 2 < len && bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
                i += 3;
                break;
            }
            i += 1;
        } else if bytes[i] == quote {
            i += 1;
            break;
        } else if bytes[i] == b'\n' {
            break;
        } else {
            i += 1;
        }
    }
    emit_string_with_escapes(source, start, i, base, out);
    i
}

fn scan_backtick_string(source: &str, start: usize, base: usize, out: &mut TokenSink) -> usize {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = start + 1;
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 2;
        } else if bytes[i] == b'`' {
            i += 1;
            break;
        } else {
            i += 1;
        }
    }
    emit_string_with_escapes(source, start, i, base, out);
    i
}

fn emit_string_with_escapes(
    source: &str,
    start: usize,
    end: usize,
    base: usize,
    out: &mut TokenSink,
) {
    let bytes = source.as_bytes();
    let mut seg_start = start;
    let mut i = start;
    while i < end {
        if bytes[i] == b'\\' && i + 1 < end {
            if i > seg_start {
                out.push(base + seg_start..base + i, TokenKind::String);
            }
            let esc_end = match bytes[i + 1] {
                b'u' if bytes.get(i + 2) == Some(&b'{') => {
                    let mut j = i + 3;
                    while j < end && bytes[j].is_ascii_hexdigit() {
                        j += 1;
                    }
                    if j < end && bytes[j] == b'}' {
                        j + 1
                    } else {
                        (i + 2).min(end)
                    }
                }
                b'u' => (i + 6).min(end),
                b'x' => (i + 4).min(end),
                _ => {
                    let next_len = source[i + 1..end].chars().next().map_or(1, char::len_utf8);
                    (i + 1 + next_len).min(end)
                }
            };
            out.push(base + i..base + esc_end, TokenKind::Escape);
            i = esc_end;
            seg_start = i;
        } else {
            i += 1;
        }
    }
    if seg_start < end {
        out.push(base + seg_start..base + end, TokenKind::String);
    }
}

fn scan_number(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;
    if bytes[i] == b'0' && i + 1 < len {
        match bytes[i + 1] {
            b'x' | b'X' => {
                i += 2;
                while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                    i += 1;
                }
                return scan_number_suffix(bytes, i);
            }
            b'b' | b'B' => {
                i += 2;
                while i < len && matches!(bytes[i], b'0' | b'1' | b'_') {
                    i += 1;
                }
                return scan_number_suffix(bytes, i);
            }
            b'o' | b'O' => {
                i += 2;
                while i < len && (matches!(bytes[i], b'0'..=b'7' | b'_')) {
                    i += 1;
                }
                return scan_number_suffix(bytes, i);
            }
            _ => {}
        }
    }
    while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if i < len && bytes[i] == b'.' && bytes.get(i + 1) != Some(&b'.') {
        i += 1;
        while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    if i < len && matches!(bytes[i], b'e' | b'E') {
        let mut j = i + 1;
        if j < len && matches!(bytes[j], b'+' | b'-') {
            j += 1;
        }
        if j < len && bytes[j].is_ascii_digit() {
            i = j;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                i += 1;
            }
        }
    }
    scan_number_suffix(bytes, i)
}

fn scan_number_suffix(bytes: &[u8], mut i: usize) -> usize {
    if i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
            i += 1;
        }
    }
    i
}

fn tokenize_json(source: &str, base: usize, out: &mut TokenSink) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Comment);
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            let start = i;
            i += 2;
            while i + 1 < len && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(len);
            out.push(base + start..base + i, TokenKind::Comment);
            continue;
        }
        if b == b'"' || b == b'\'' {
            let start = i;
            let end = scan_plain_quoted_end(bytes, i, b);
            let mut look = end;
            while look < len && bytes[look].is_ascii_whitespace() {
                look += 1;
            }
            if look < len && bytes[look] == b':' {
                out.push(base + start..base + end, TokenKind::Property);
            } else {
                emit_string_with_escapes(source, start, end, base, out);
            }
            i = end;
            continue;
        }
        if b.is_ascii_digit() || (b == b'-' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            if b == b'-' {
                i += 1;
            }
            i = scan_number(bytes, i);
            out.push(base + start..base + i, TokenKind::Number);
            continue;
        }
        if is_ident_start(b) {
            let start = i;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &source[start..i];
            let kind = match word {
                "true" | "false" | "null" => TokenKind::Constant,
                _ => TokenKind::Property,
            };
            out.push(base + start..base + i, kind);
            continue;
        }
        if b.is_ascii_punctuation() {
            out.push(base + i..base + i + 1, TokenKind::Operator);
            i += 1;
            continue;
        }
        let ch_len = source[i..].chars().next().map_or(1, char::len_utf8);
        out.push(base + i..base + i + ch_len, TokenKind::Plain);
        i += ch_len;
    }
}

fn tokenize_yaml(source: &str, base: usize, out: &mut TokenSink) {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();
        if trimmed.starts_with('#') {
            out.push(
                base + offset..base + offset + content.len(),
                TokenKind::Comment,
            );
        } else if trimmed == "---" || trimmed == "..." {
            out.push(
                base + offset + indent..base + offset + content.len(),
                TokenKind::Keyword,
            );
        } else {
            let mut pos = indent;
            if let Some(rest) = trimmed.strip_prefix("- ") {
                out.push(
                    base + offset + pos..base + offset + pos + 1,
                    TokenKind::Operator,
                );
                pos += 2;
                let extra = rest.len() - rest.trim_start().len();
                pos += extra;
            }
            let remainder = &content[pos..];
            if let Some(colon) = find_yaml_colon(remainder) {
                let key_end = pos + colon;
                out.push(
                    base + offset + pos..base + offset + key_end,
                    TokenKind::Property,
                );
                out.push(
                    base + offset + key_end..base + offset + key_end + 1,
                    TokenKind::Operator,
                );
                tokenize_yaml_value(&content[key_end + 1..], base + offset + key_end + 1, out);
            } else {
                tokenize_yaml_value(remainder, base + offset + pos, out);
            }
        }
        offset += line.len();
    }
}

fn find_yaml_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes.is_empty() || matches!(bytes[0], b'"' | b'\'' | b'#' | b'[' | b'{') {
        return None;
    }
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b':' {
            let next = bytes.get(idx + 1).copied();
            if next.is_none() || next == Some(b' ') || next == Some(b'\t') {
                return Some(idx);
            }
        }
        if b == b'#' {
            return None;
        }
    }
    None
}

fn tokenize_yaml_value(value: &str, base: usize, out: &mut TokenSink) {
    let bytes = value.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= len {
        return;
    }
    if bytes[i] == b'#' {
        out.push(base + i..base + len, TokenKind::Comment);
        return;
    }
    // Check for inline comment outside quotes
    let mut comment_pos = len;
    let mut in_quote = 0u8;
    let mut j = i;
    while j < len {
        let b = bytes[j];
        if in_quote != 0 {
            if b == b'\\' && in_quote == b'"' && j + 1 < len {
                j += 2;
                continue;
            }
            if b == in_quote {
                in_quote = 0;
            }
        } else if b == b'"' || b == b'\'' {
            in_quote = b;
        } else if b == b'#' && (j == 0 || bytes[j - 1].is_ascii_whitespace()) {
            comment_pos = j;
            break;
        }
        j += 1;
    }
    let raw = value[i..comment_pos].trim_end();
    let raw_end = i + raw.len();
    if !raw.is_empty() {
        if raw.starts_with('"') || raw.starts_with('\'') {
            emit_string_with_escapes(value, i, raw_end, base, out);
        } else if raw.starts_with('&') || raw.starts_with('*') || raw.starts_with('!') {
            out.push(base + i..base + raw_end, TokenKind::Type);
        } else if matches!(
            raw,
            "true" | "false" | "True" | "False" | "null" | "Null" | "~" | "yes" | "no"
        ) {
            out.push(base + i..base + raw_end, TokenKind::Constant);
        } else if is_plain_number(raw) {
            out.push(base + i..base + raw_end, TokenKind::Number);
        } else if matches!(raw, "|" | ">" | "|-" | ">-" | "|+" | ">+") {
            out.push(base + i..base + raw_end, TokenKind::Operator);
        } else {
            out.push(base + i..base + raw_end, TokenKind::String);
        }
    }
    if comment_pos < len {
        out.push(base + comment_pos..base + len, TokenKind::Comment);
    }
}

fn tokenize_toml_ini(source: &str, base: usize, is_toml: bool, out: &mut TokenSink) {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();
        if trimmed.starts_with('#') || (!is_toml && trimmed.starts_with(';')) {
            out.push(
                base + offset..base + offset + content.len(),
                TokenKind::Comment,
            );
        } else if trimmed.starts_with('[') {
            if let Some(close) = content.rfind(']') {
                out.push(
                    base + offset + indent..base + offset + close + 1,
                    TokenKind::Tag,
                );
                if let Some(hash) = content[close + 1..].find('#') {
                    out.push(
                        base + offset + close + 1 + hash..base + offset + content.len(),
                        TokenKind::Comment,
                    );
                }
            } else {
                out.push(
                    base + offset + indent..base + offset + content.len(),
                    TokenKind::Tag,
                );
            }
        } else if let Some(eq) = content.find('=') {
            let key_part = &content[indent..eq];
            let key_trimmed = key_part.trim_end();
            out.push(
                base + offset + indent..base + offset + indent + key_trimmed.len(),
                TokenKind::Property,
            );
            out.push(
                base + offset + eq..base + offset + eq + 1,
                TokenKind::Operator,
            );
            tokenize_toml_value(&content[eq + 1..], base + offset + eq + 1, out);
        } else {
            tokenize_toml_value(content, base + offset, out);
        }
        offset += line.len();
    }
}

fn tokenize_toml_value(value: &str, base: usize, out: &mut TokenSink) {
    let bytes = value.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'#' {
            out.push(base + i..base + len, TokenKind::Comment);
            return;
        }
        if b == b'"' || b == b'\'' {
            i = scan_string(value, i, b, true, base, None, out);
            continue;
        }
        if b.is_ascii_digit()
            || (matches!(b, b'-' | b'+') && i + 1 < len && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric()
                    || matches!(bytes[i], b'.' | b'_' | b'-' | b':' | b'+'))
            {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Number);
            continue;
        }
        if is_ident_start(b) {
            let start = i;
            while i < len && (is_ident_continue(bytes[i]) || bytes[i] == b'-') {
                i += 1;
            }
            let word = &value[start..i];
            let mut look = i;
            while look < len && matches!(bytes[look], b' ' | b'\t') {
                look += 1;
            }
            let kind = if look < len && bytes[look] == b'=' {
                TokenKind::Property
            } else if matches!(word, "true" | "false") {
                TokenKind::Constant
            } else {
                TokenKind::Plain
            };
            out.push(base + start..base + i, kind);
            continue;
        }
        if b.is_ascii_punctuation() {
            out.push(base + i..base + i + 1, TokenKind::Operator);
            i += 1;
            continue;
        }
        let ch_len = value[i..].chars().next().map_or(1, char::len_utf8);
        out.push(base + i..base + i + ch_len, TokenKind::Plain);
        i += ch_len;
    }
}

fn tokenize_html(source: &str, base: usize, out: &mut TokenSink) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i..].starts_with(b"<!--") {
            let start = i;
            i += 4;
            while i + 2 < len && !bytes[i..].starts_with(b"-->") {
                i += 1;
            }
            i = (i + 3).min(len);
            out.push(base + start..base + i, TokenKind::Comment);
            continue;
        }
        if bytes[i..].starts_with(b"<!") || bytes[i..].starts_with(b"<?") {
            let start = i;
            while i < len && bytes[i] != b'>' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Property);
            continue;
        }
        if bytes[i] == b'<' {
            let open_start = i;
            i += 1;
            let closing = i < len && bytes[i] == b'/';
            if closing {
                i += 1;
            }
            out.push(base + open_start..base + i, TokenKind::Operator);
            let name_start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_' | b':'))
            {
                i += 1;
            }
            let tag_name = &source[name_start..i];
            if i > name_start {
                out.push(base + name_start..base + i, TokenKind::Tag);
            }
            let mut self_closing = false;
            while i < len && bytes[i] != b'>' {
                if bytes[i].is_ascii_whitespace() {
                    i += 1;
                    continue;
                }
                if bytes[i] == b'/' {
                    self_closing = true;
                    out.push(base + i..base + i + 1, TokenKind::Operator);
                    i += 1;
                    continue;
                }
                if bytes[i] == b'=' {
                    out.push(base + i..base + i + 1, TokenKind::Operator);
                    i += 1;
                    continue;
                }
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    let quote = bytes[i];
                    let start = i;
                    let end = scan_plain_quoted_end(bytes, i, quote);
                    out.push(base + start..base + end, TokenKind::String);
                    i = end;
                    continue;
                }
                let attr_start = i;
                while i < len
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'=' | b'>' | b'/' | b'"' | b'\'')
                {
                    i += 1;
                }
                if i > attr_start {
                    out.push(base + attr_start..base + i, TokenKind::Property);
                } else {
                    i += 1;
                }
            }
            if i < len && bytes[i] == b'>' {
                out.push(base + i..base + i + 1, TokenKind::Operator);
                i += 1;
            }
            // Embedded <script> / <style> bodies
            if !closing && !self_closing {
                if tag_name.eq_ignore_ascii_case("script") {
                    if let Some(rel_end) = find_ascii_case_insensitive(&source[i..], "</script") {
                        tokenize_code(
                            &source[i..i + rel_end],
                            base + i,
                            lang_spec(LanguageKind::TypeScript),
                            out,
                        );
                        i += rel_end;
                    }
                } else if tag_name.eq_ignore_ascii_case("style")
                    && let Some(rel_end) = find_ascii_case_insensitive(&source[i..], "</style")
                {
                    tokenize_css(&source[i..i + rel_end], base + i, out);
                    i += rel_end;
                }
            }
            continue;
        }
        if bytes[i] == b'&' {
            let start = i;
            let mut j = i + 1;
            while j < len
                && j < start + 12
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'#')
            {
                j += 1;
            }
            if j > start + 1 && j < len && bytes[j] == b';' {
                out.push(base + start..base + j + 1, TokenKind::Escape);
                i = j + 1;
                continue;
            }
        }
        let ch_len = source[i..].chars().next().map_or(1, char::len_utf8);
        i += ch_len;
    }
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    h.windows(n.len())
        .position(|window| window.eq_ignore_ascii_case(n))
}

fn tokenize_css(source: &str, base: usize, out: &mut TokenSink) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut brace_depth = 0usize;

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            let start = i;
            i += 2;
            while i + 1 < len && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(len);
            out.push(base + start..base + i, TokenKind::Comment);
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Comment);
            continue;
        }
        if b == b'{' {
            brace_depth += 1;
            out.push(base + i..base + i + 1, TokenKind::Operator);
            i += 1;
            continue;
        }
        if b == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            out.push(base + i..base + i + 1, TokenKind::Operator);
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let end = scan_plain_quoted_end(bytes, i, b);
            out.push(base + i..base + end, TokenKind::String);
            i = end;
            continue;
        }
        if b == b'@' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_')) {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Keyword);
            continue;
        }
        if b == b'!' && bytes[i..].starts_with(b"!important") {
            out.push(base + i..base + i + 10, TokenKind::Keyword);
            i += 10;
            continue;
        }
        // Hex color #fff / #1a2b3c4d or ID selector
        if b == b'#'
            && i + 1 < len
            && (bytes[i + 1].is_ascii_alphanumeric() || bytes[i + 1] == b'-')
        {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_')) {
                i += 1;
            }
            let kind = if brace_depth > 0 {
                TokenKind::Number
            } else {
                TokenKind::Type
            };
            out.push(base + start..base + i, kind);
            continue;
        }
        // CSS custom property --var-name
        if bytes[i..].starts_with(b"--") {
            let start = i;
            i += 2;
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_')) {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::Property);
            continue;
        }
        // Class selector .foo or number .5rem
        if b == b'.' {
            if i + 1 < len && bytes[i + 1].is_ascii_digit() {
                let start = i;
                i = scan_css_number(bytes, i);
                out.push(base + start..base + i, TokenKind::Number);
                continue;
            }
            if brace_depth == 0
                && i + 1 < len
                && (bytes[i + 1].is_ascii_alphabetic() || matches!(bytes[i + 1], b'-' | b'_'))
            {
                let start = i;
                i += 1;
                while i < len
                    && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_'))
                {
                    i += 1;
                }
                out.push(base + start..base + i, TokenKind::Tag);
                continue;
            }
        }
        if b.is_ascii_digit()
            || (b == b'-' && i + 1 < len && (bytes[i + 1].is_ascii_digit() || bytes[i + 1] == b'.'))
        {
            let start = i;
            i = scan_css_number(bytes, i);
            out.push(base + start..base + i, TokenKind::Number);
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' || b == b'-' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_')) {
                i += 1;
            }
            let mut look = i;
            while look < len && matches!(bytes[look], b' ' | b'\t') {
                look += 1;
            }
            let kind = if look < len && bytes[look] == b'(' {
                TokenKind::Function
            } else if brace_depth > 0 && look < len && bytes[look] == b':' {
                TokenKind::Property
            } else if brace_depth == 0 {
                TokenKind::Tag
            } else {
                TokenKind::Constant
            };
            out.push(base + start..base + i, kind);
            continue;
        }
        if b.is_ascii_punctuation() {
            out.push(base + i..base + i + 1, TokenKind::Operator);
            i += 1;
            continue;
        }
        let ch_len = source[i..].chars().next().map_or(1, char::len_utf8);
        i += ch_len;
    }
}

fn scan_css_number(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;
    if i < len && bytes[i] == b'-' {
        i += 1;
    }
    while i < len && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < len && bytes[i] == b'.' {
        i += 1;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    // CSS unit like px, rem, em, %, vh, ms, s, deg
    if i < len && bytes[i] == b'%' {
        i += 1;
    } else {
        while i < len && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
    }
    i
}

fn tokenize_markdown(source: &str, base: usize, out: &mut TokenSink) {
    let mut offset = 0;
    let mut fence_lang: Option<LanguageKind> = None;
    let mut fence_buf_start: Option<usize> = None;

    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if let Some(start) = fence_buf_start.take() {
                if let Some(lang) = fence_lang.take() {
                    tokenize_into_slice(&source[start..offset], base + start, lang, out);
                }
                out.push(
                    base + offset + indent..base + offset + content.len(),
                    TokenKind::Property,
                );
            } else {
                let lang_name = trimmed[3..].trim();
                fence_lang = resolve_language(lang_name);
                fence_buf_start = Some(offset + line.len());
                out.push(
                    base + offset + indent..base + offset + content.len(),
                    TokenKind::Property,
                );
            }
            offset += line.len();
            continue;
        }

        if fence_buf_start.is_some() {
            offset += line.len();
            continue;
        }

        if trimmed.starts_with('#') {
            out.push(
                base + offset + indent..base + offset + content.len(),
                TokenKind::Tag,
            );
        } else if trimmed.starts_with('>') {
            out.push(
                base + offset + indent..base + offset + content.len(),
                TokenKind::Comment,
            );
        } else {
            let mut pos = indent;
            if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
                out.push(
                    base + offset + pos..base + offset + pos + 1,
                    TokenKind::Keyword,
                );
                pos += 2;
            }
            tokenize_markdown_inline(&content[pos..], base + offset + pos, out);
        }
        offset += line.len();
    }

    if let Some(start) = fence_buf_start
        && let Some(lang) = fence_lang
    {
        tokenize_into_slice(&source[start..], base + start, lang, out);
    }
}

fn tokenize_into_slice(source: &str, base: usize, kind: LanguageKind, out: &mut TokenSink) {
    match kind {
        LanguageKind::Json => tokenize_json(source, base, out),
        LanguageKind::Yaml => tokenize_yaml(source, base, out),
        LanguageKind::Toml => tokenize_toml_ini(source, base, true, out),
        LanguageKind::Ini => tokenize_toml_ini(source, base, false, out),
        LanguageKind::Html => tokenize_html(source, base, out),
        LanguageKind::Css => tokenize_css(source, base, out),
        LanguageKind::Diff => tokenize_diff(source, base, out),
        LanguageKind::Makefile => tokenize_makefile(source, base, out),
        LanguageKind::Markdown => out.push(base..base + source.len(), TokenKind::Plain),
        _ => tokenize_code(source, base, lang_spec(kind), out),
    }
}

fn tokenize_markdown_inline(line: &str, base: usize, out: &mut TokenSink) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            out.push(base + start..base + i, TokenKind::String);
            continue;
        }
        if bytes[i] == b'['
            && let Some(close) = line[i + 1..].find("](")
        {
            let label_end = i + 1 + close;
            if let Some(paren_close) = line[label_end + 2..].find(')') {
                let url_end = label_end + 2 + paren_close;
                out.push(base + i..base + label_end + 1, TokenKind::Function);
                out.push(base + label_end + 1..base + url_end + 1, TokenKind::Number);
                i = url_end + 1;
                continue;
            }
        }
        if bytes[i..].starts_with(b"**")
            && let Some(close) = line[i + 2..].find("**")
        {
            let end = i + 2 + close + 2;
            out.push(base + i..base + end, TokenKind::Type);
            i = end;
            continue;
        }
        let ch_len = line[i..].chars().next().map_or(1, char::len_utf8);
        i += ch_len;
    }
}

fn tokenize_diff(source: &str, base: usize, out: &mut TokenSink) {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let end = offset + line.len();
        let kind = if line.starts_with("@@") {
            TokenKind::Property
        } else if ["+++ ", "--- ", "diff ", "index ", "*** "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            TokenKind::Comment
        } else if line.starts_with('+') {
            TokenKind::Added
        } else if line.starts_with('-') {
            TokenKind::Removed
        } else {
            TokenKind::Plain
        };
        out.push(base + offset..base + end, kind);
        offset = end;
    }
}

fn tokenize_makefile(source: &str, base: usize, out: &mut TokenSink) {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        if trimmed.starts_with('#') {
            out.push(
                base + offset..base + offset + content.len(),
                TokenKind::Comment,
            );
        } else if content.starts_with('\t') {
            tokenize_code(
                &content[1..],
                base + offset + 1,
                lang_spec(LanguageKind::Shell),
                out,
            );
        } else if let Some(colon) = content.find(':')
            && !content[..colon].contains('=')
        {
            out.push(base + offset..base + offset + colon, TokenKind::Function);
            out.push(
                base + offset + colon..base + offset + colon + 1,
                TokenKind::Operator,
            );
        } else if let Some(eq) = content.find('=') {
            let key_end = content[..eq].trim_end_matches([':', '?', '+', ' ']).len();
            out.push(base + offset..base + offset + key_end, TokenKind::Property);
            out.push(
                base + offset + key_end..base + offset + eq + 1,
                TokenKind::Operator,
            );
            tokenize_code(
                &content[eq + 1..],
                base + offset + eq + 1,
                lang_spec(LanguageKind::Shell),
                out,
            );
        }
        offset += line.len();
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_plain_number(text: &str) -> bool {
    let s = text.strip_prefix('-').unwrap_or(text);
    !s.is_empty()
        && s.as_bytes()[0].is_ascii_digit()
        && s.bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'e' | b'E' | b'+' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn assert_valid_coverage(source: &str, tokens: &[Token]) {
        if source.is_empty() {
            assert!(tokens.is_empty());
            return;
        }
        let mut rebuilt = String::new();
        let mut cursor = 0;
        for token in tokens {
            assert_eq!(token.range.start, cursor, "gap or overlap in {tokens:?}");
            assert!(token.range.start < token.range.end);
            rebuilt.push_str(&source[token.range.clone()]);
            cursor = token.range.end;
        }
        assert_eq!(cursor, source.len());
        assert_eq!(rebuilt, source);
    }

    #[test]
    fn highlights_common_modern_languages_with_multiple_token_kinds() {
        let samples = [
            (
                "rs",
                "#[derive(Debug)]\npub fn hello<'a>(name: &'a str) -> String {\n    // 中文注释\n    format!(\"hi\\n{name}\")\n}\n",
            ),
            (
                "tsx",
                "export const App = ({ count }: { count: number }) => {\n  // comment\n  return <Button disabled={false}>{count + 1}</Button>;\n};\n",
            ),
            (
                "ts",
                "interface User { id: number; name: string }\nconst active: boolean = true;\n",
            ),
            (
                "toml",
                "[package]\nname = \"narolink\"\nversion = \"0.0.0\"\n# comment\nstrip = true\n",
            ),
            (
                "json",
                "{\"name\": \"Naro\", \"count\": 42, \"ok\": true}\n",
            ),
            (
                "py",
                "@decorator\ndef greet(name: str) -> str:\n    # hi\n    return f\"hello {name}\"\n",
            ),
            (
                "go",
                "package main\nfunc main() { const n = 10; println(\"ok\", n) }\n",
            ),
            (
                "css",
                ".card { background-color: #ffffff; padding: 12px; /* note */ }\n",
            ),
            (
                "html",
                "<!-- note --><div class=\"hero\"><script>const x = 1;</script></div>\n",
            ),
            (
                "sh",
                "#!/bin/sh\nexport APP=\"Naro\"\nif [ -n \"$APP\" ]; then echo 1; fi\n",
            ),
            (
                "swift",
                "import AppKit\n@main struct App { let count: Int = 3 }\n",
            ),
            (
                "yaml",
                "services:\n  relay:\n    image: \"naro:latest\"\n    ports: 8080\n    restart: true # comment\n",
            ),
            (
                "md",
                "# Title\n\nUse `Naro` and [docs](README.md).\n\n```ts\nconst x = 1;\n```\n",
            ),
        ];

        for (lang, source) in samples {
            let tokens = tokenize(source, lang);
            assert_valid_coverage(source, &tokens);
            let kinds: HashSet<_> = tokens.iter().map(|token| token.kind).collect();
            assert!(
                kinds.len() >= 4,
                "expected rich highlighting for {lang}, got {kinds:?}"
            );
        }
    }

    #[test]
    fn detects_extensionless_and_special_filenames() {
        assert_eq!(
            detect_syntax_from_path(Path::new("Dockerfile")),
            "dockerfile"
        );
        assert_eq!(detect_syntax_from_path(Path::new("Makefile")), "makefile");
        assert_eq!(detect_syntax_from_path(Path::new("Cargo.lock")), "toml");
        assert_eq!(detect_syntax_from_path(Path::new(".zshrc")), "sh");
        assert_eq!(detect_syntax_from_path(Path::new(".env.local")), "env");
        assert_eq!(detect_syntax_from_path(Path::new("main.RS")), "rs");
        assert_eq!(detect_syntax_from_path(Path::new("LICENSE")), "");
    }

    #[test]
    fn highlight_html_escapes_markup_safely() {
        let html = highlight_html("const tag = \"<script>&\";", "ts").unwrap();
        assert!(html.contains("&lt;script&gt;&amp;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("class=\"tok-keyword\""));
    }
}
