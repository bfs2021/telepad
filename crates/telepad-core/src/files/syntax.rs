//! Syntax colours for the editor (#64): the language from the file name (or
//! a `#!` line) and a small line-based lexer per language. Not a parser:
//! comments, strings, numbers, keywords, keys / sections and variables.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    Plain,
    Shell,
    Yaml,
    Json,
    Toml,
    Ini,
    Env,
    Dockerfile,
    Nginx,
    Python,
    JavaScript,
    TypeScript,
    Php,
    Sql,
    Markdown,
    Rust,
}

impl Lang {
    pub fn name(self) -> &'static str {
        match self {
            Lang::Plain => "Plain text",
            Lang::Shell => "Shell",
            Lang::Yaml => "YAML",
            Lang::Json => "JSON",
            Lang::Toml => "TOML",
            Lang::Ini => "INI / conf",
            Lang::Env => ".env",
            Lang::Dockerfile => "Dockerfile",
            Lang::Nginx => "nginx",
            Lang::Python => "Python",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Php => "PHP",
            Lang::Sql => "SQL",
            Lang::Markdown => "Markdown",
            Lang::Rust => "Rust",
        }
    }

    /// By file name and folder, else by the first line (`#!/bin/sh`).
    pub fn detect(path: &str, first_line: &str) -> Lang {
        let p = path.replace('\\', "/").to_lowercase();
        let name = p.rsplit('/').next().unwrap_or("");
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        if name == "dockerfile" || name.starts_with("dockerfile.") || ext == "dockerfile" {
            return Lang::Dockerfile;
        }
        if name == ".env" || name.starts_with(".env.") || ext == "env" {
            return Lang::Env;
        }
        if (ext == "conf" && (name.contains("nginx") || p.contains("/nginx/")))
            || p.contains("/nginx/sites-")
            || p.contains("/nginx/conf.d/")
        {
            return Lang::Nginx;
        }
        let by_ext = match ext {
            "sh" | "bash" | "zsh" | "ksh" => Lang::Shell,
            "yml" | "yaml" => Lang::Yaml,
            "json" => Lang::Json,
            "toml" => Lang::Toml,
            "ini" | "conf" | "cfg" | "cnf" | "properties" | "service" | "timer" => Lang::Ini,
            "py" => Lang::Python,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "tsx" => Lang::TypeScript,
            "php" => Lang::Php,
            "sql" => Lang::Sql,
            "md" | "markdown" => Lang::Markdown,
            "rs" => Lang::Rust,
            _ => Lang::Plain,
        };
        if by_ext != Lang::Plain {
            return by_ext;
        }
        if matches!(
            name,
            ".bashrc" | ".profile" | ".bash_profile" | ".bash_aliases" | ".zshrc" | ".bash_logout"
        ) {
            return Lang::Shell;
        }
        if let Some(sb) = first_line.strip_prefix("#!") {
            if sb.contains("python") {
                return Lang::Python;
            }
            if sb.contains("node") {
                return Lang::JavaScript;
            }
            if ["sh", "bash", "zsh", "ksh", "dash", "ash"]
                .iter()
                .any(|s| sb.trim_end().ends_with(&format!("/{s}")) || sb.contains(&format!(" {s}")))
            {
                return Lang::Shell;
            }
        }
        Lang::Plain
    }

    /// Every language the editor colours (for the language menu).
    pub const ALL: [Lang; 16] = [
        Lang::Plain,
        Lang::Shell,
        Lang::Yaml,
        Lang::Json,
        Lang::Toml,
        Lang::Ini,
        Lang::Env,
        Lang::Dockerfile,
        Lang::Nginx,
        Lang::Python,
        Lang::JavaScript,
        Lang::TypeScript,
        Lang::Php,
        Lang::Sql,
        Lang::Markdown,
        Lang::Rust,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tok {
    Plain,
    Comment,
    Str,
    Num,
    Keyword,
    /// YAML / JSON / INI / TOML key, `.env` name, nginx directive.
    Key,
    /// `[section]`, Markdown heading.
    Heading,
    /// `$var`, `${var}`.
    Var,
}

struct Spec {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    triple: bool,
    kw: &'static [&'static str],
    kw_ci: bool,
    vars: bool,
}

const SHELL_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "function", "return", "export", "local", "readonly", "set", "unset", "shift", "exit",
    "break", "continue", "source", "alias", "echo", "printf", "read", "cd", "test", "true",
    "false", "sudo",
];
const PY_KW: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "self", "print",
];
const JS_KW: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "async",
    "await",
    "of",
    "static",
    "get",
    "set",
    "from",
];
const TS_KW: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "async",
    "await",
    "of",
    "static",
    "get",
    "set",
    "from",
    "interface",
    "type",
    "enum",
    "implements",
    "private",
    "public",
    "protected",
    "readonly",
    "declare",
    "namespace",
    "abstract",
    "any",
    "number",
    "string",
    "boolean",
    "never",
    "unknown",
    "as",
    "keyof",
];
const PHP_KW: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
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
    "endforeach",
    "endif",
    "endwhile",
    "extends",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
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
    "null",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "true",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "yield",
];
const SQL_KW: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "not",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "alter",
    "drop",
    "index",
    "primary",
    "key",
    "foreign",
    "references",
    "join",
    "left",
    "right",
    "inner",
    "outer",
    "full",
    "on",
    "as",
    "group",
    "by",
    "order",
    "having",
    "limit",
    "offset",
    "union",
    "all",
    "distinct",
    "null",
    "is",
    "in",
    "like",
    "between",
    "exists",
    "case",
    "when",
    "then",
    "else",
    "end",
    "default",
    "unique",
    "check",
    "constraint",
    "view",
    "database",
    "if",
    "begin",
    "commit",
    "rollback",
    "transaction",
    "grant",
    "revoke",
    "int",
    "integer",
    "varchar",
    "text",
    "char",
    "boolean",
    "date",
    "timestamp",
    "datetime",
    "serial",
    "bigint",
    "auto_increment",
    "asc",
    "desc",
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "cascade",
    "use",
    "show",
    "truncate",
];
const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "Some", "None", "Ok", "Err",
];
const YAML_KW: &[&str] = &["true", "false", "null", "yes", "no", "on", "off", "~"];
const BOOL_KW: &[&str] = &["true", "false"];
const JSON_KW: &[&str] = &["true", "false", "null"];
const NGINX_KW: &[&str] = &["on", "off"];
const DOCKER_KW: &[&str] = &[
    "from",
    "run",
    "cmd",
    "label",
    "maintainer",
    "expose",
    "env",
    "add",
    "copy",
    "entrypoint",
    "volume",
    "user",
    "workdir",
    "arg",
    "onbuild",
    "stopsignal",
    "healthcheck",
    "shell",
];

fn spec(lang: Lang) -> Spec {
    let s = Spec {
        line: &[],
        block: None,
        quotes: &['"', '\''],
        triple: false,
        kw: &[],
        kw_ci: false,
        vars: false,
    };
    match lang {
        Lang::Plain | Lang::Markdown => Spec { quotes: &[], ..s },
        Lang::Shell | Lang::Dockerfile | Lang::Env => Spec {
            line: &["#"],
            quotes: &['"', '\'', '`'],
            kw: SHELL_KW,
            vars: true,
            ..s
        },
        Lang::Yaml => Spec {
            line: &["#"],
            kw: YAML_KW,
            kw_ci: true,
            ..s
        },
        Lang::Json => Spec {
            quotes: &['"'],
            kw: JSON_KW,
            ..s
        },
        Lang::Toml => Spec {
            line: &["#"],
            triple: true,
            kw: BOOL_KW,
            ..s
        },
        Lang::Ini => Spec {
            line: &["#", ";"],
            quotes: &['"'],
            ..s
        },
        Lang::Nginx => Spec {
            line: &["#"],
            kw: NGINX_KW,
            vars: true,
            ..s
        },
        Lang::Python => Spec {
            line: &["#"],
            triple: true,
            kw: PY_KW,
            ..s
        },
        Lang::JavaScript => Spec {
            line: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            kw: JS_KW,
            ..s
        },
        Lang::TypeScript => Spec {
            line: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            kw: TS_KW,
            ..s
        },
        Lang::Php => Spec {
            line: &["//", "#"],
            block: Some(("/*", "*/")),
            kw: PHP_KW,
            kw_ci: true,
            vars: true,
            ..s
        },
        Lang::Sql => Spec {
            line: &["--", "#"],
            block: Some(("/*", "*/")),
            kw: SQL_KW,
            kw_ci: true,
            ..s
        },
        Lang::Rust => Spec {
            line: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"'],
            kw: RUST_KW,
            ..s
        },
    }
}

/// What carries over from one line to the next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Carry {
    None,
    Block,
    /// Inside a `"""` / `'''` string.
    Triple(char),
    /// Inside a Markdown ``` fence.
    Fence,
}

struct Out {
    spans: Vec<(Range<usize>, Tok)>,
}

impl Out {
    fn push(&mut self, r: Range<usize>, t: Tok) {
        if r.is_empty() {
            return;
        }
        if let Some((last, lt)) = self.spans.last_mut()
            && *lt == t
            && last.end == r.start
        {
            last.end = r.end;
            return;
        }
        self.spans.push((r, t));
    }
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Spans covering the whole text, in order (byte ranges).
pub fn highlight(lang: Lang, text: &str) -> Vec<(Range<usize>, Tok)> {
    let sp = spec(lang);
    let mut out = Out { spans: Vec::new() };
    let mut carry = Carry::None;
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        carry = line_spans(lang, &sp, line, start, carry, &mut out);
        start += line.len();
    }
    out.spans
}

/// The key / section / directive at the start of a line: (end byte, kind).
fn line_head(lang: Lang, line: &str) -> Option<(usize, Tok)> {
    let body = line.trim_end_matches(['\n', '\r']);
    let lead = body.len() - body.trim_start().len();
    let t = &body[lead..];
    if t.is_empty() {
        return None;
    }
    match lang {
        Lang::Ini | Lang::Toml if t.starts_with('[') => {
            let end = t.find(']').map_or(t.len(), |i| i + 1);
            Some((lead + end, Tok::Heading))
        }
        Lang::Ini | Lang::Toml | Lang::Env => {
            if t.starts_with(['#', ';']) {
                return None;
            }
            let k = t.strip_prefix("export ").map_or(0, |_| 7);
            let eq = t
                .find('=')
                .or_else(|| (lang == Lang::Ini).then(|| t.find(':')).flatten())?;
            let key = &t[k..eq];
            let ok = !key.trim().is_empty()
                && key.trim().chars().all(|c| {
                    ident_char(c) || matches!(c, '.' | '-' | '"' | '\'' | ' ' | '/' | '@')
                });
            ok.then_some((lead + eq, Tok::Key))
        }
        Lang::Yaml => {
            let t2 = t.strip_prefix("- ").unwrap_or(t);
            let off = lead + (t.len() - t2.len());
            if t2.starts_with('#') {
                return None;
            }
            let colon = t2
                .char_indices()
                .find(|&(i, c)| {
                    c == ':' && t2[i + 1..].chars().next().is_none_or(char::is_whitespace)
                })
                .map(|(i, _)| i)?;
            let key = &t2[..colon];
            (!key.contains(" #") && !key.starts_with(['{', '['])).then_some((off + colon, Tok::Key))
        }
        Lang::Nginx => {
            if t.starts_with(['#', '}', '{']) {
                return None;
            }
            let end = t
                .find(|c: char| c.is_whitespace() || c == ';' || c == '{')
                .unwrap_or(t.len());
            Some((lead + end, Tok::Key))
        }
        Lang::Dockerfile => {
            let end = t.find(char::is_whitespace).unwrap_or(t.len());
            DOCKER_KW
                .contains(&t[..end].to_lowercase().as_str())
                .then_some((lead + end, Tok::Keyword))
        }
        _ => None,
    }
}

fn line_spans(
    lang: Lang,
    sp: &Spec,
    line: &str,
    base: usize,
    carry: Carry,
    out: &mut Out,
) -> Carry {
    let n = line.len();
    if lang == Lang::Markdown {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            out.push(base..base + n, Tok::Str);
            return if carry == Carry::Fence {
                Carry::None
            } else {
                Carry::Fence
            };
        }
        if carry == Carry::Fence {
            out.push(base..base + n, Tok::Str);
            return carry;
        }
        if t.starts_with('#') {
            out.push(base..base + n, Tok::Heading);
            return Carry::None;
        }
        // `inline code`
        let mut i = 0;
        while i < n {
            if line[i..].starts_with('`')
                && let Some(j) = line[i + 1..].find('`')
            {
                out.push(base + i..base + i + j + 2, Tok::Str);
                i += j + 2;
                continue;
            }
            let c = line[i..].chars().next().map_or(1, char::len_utf8);
            out.push(base + i..base + i + c, Tok::Plain);
            i += c;
        }
        return Carry::None;
    }
    let mut i = 0;
    let mut carry = carry;
    // Continue a block comment / triple string from the line before.
    match carry {
        Carry::Block => {
            let end = sp.block.map(|(_, e)| e).unwrap_or("*/");
            match line.find(end) {
                Some(j) => {
                    out.push(base..base + j + end.len(), Tok::Comment);
                    i = j + end.len();
                    carry = Carry::None;
                }
                None => {
                    out.push(base..base + n, Tok::Comment);
                    return carry;
                }
            }
        }
        Carry::Triple(q) => {
            let close: String = std::iter::repeat_n(q, 3).collect();
            match line.find(&close) {
                Some(j) => {
                    out.push(base..base + j + 3, Tok::Str);
                    i = j + 3;
                    carry = Carry::None;
                }
                None => {
                    out.push(base..base + n, Tok::Str);
                    return carry;
                }
            }
        }
        Carry::Fence | Carry::None => {}
    }
    if i == 0
        && let Some((end, tok)) = line_head(lang, line)
    {
        out.push(base..base + end, tok);
        i = end;
    }
    let bytes = line.as_bytes();
    while i < n {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap_or(' ');
        let cl = c.len_utf8();
        // Comments: `#` in shell-like languages only at a word start.
        if let Some(lc) = sp.line.iter().find(|lc| rest.starts_with(**lc)) {
            let word_start = i == 0 || bytes[i - 1].is_ascii_whitespace() || *lc != "#";
            let ini_ok = lang != Lang::Ini || line[..i].trim().is_empty();
            if word_start && ini_ok {
                out.push(base + i..base + n, Tok::Comment);
                return Carry::None;
            }
        }
        if let Some((open, close)) = sp.block
            && rest.starts_with(open)
        {
            match rest[open.len()..].find(close) {
                Some(j) => {
                    let e = i + open.len() + j + close.len();
                    out.push(base + i..base + e, Tok::Comment);
                    i = e;
                    continue;
                }
                None => {
                    out.push(base + i..base + n, Tok::Comment);
                    return Carry::Block;
                }
            }
        }
        if sp.quotes.contains(&c) {
            if sp.triple && (c == '"' || c == '\'') && rest.starts_with(&c.to_string().repeat(3)) {
                let close = c.to_string().repeat(3);
                match rest[3..].find(&close) {
                    Some(j) => {
                        let e = i + 3 + j + 3;
                        out.push(base + i..base + e, Tok::Str);
                        i = e;
                        continue;
                    }
                    None => {
                        out.push(base + i..base + n, Tok::Str);
                        return Carry::Triple(c);
                    }
                }
            }
            // To the closing quote on this line (backslash escapes, not in shell '…').
            let mut j = i + 1;
            let esc = !(c == '\'' && matches!(lang, Lang::Shell | Lang::Env | Lang::Dockerfile));
            while j < n && bytes[j] != b'\n' {
                if esc && bytes[j] == b'\\' {
                    // Skip the escaped character, whatever its length.
                    j += 1 + line[j + 1..].chars().next().map_or(0, char::len_utf8);
                    continue;
                }
                if bytes[j] == c as u8 {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let j = j.min(n);
            let tok = if lang == Lang::Json && line[j..].trim_start().starts_with(':') {
                Tok::Key
            } else {
                Tok::Str
            };
            out.push(base + i..base + j, tok);
            i = j;
            continue;
        }
        if sp.vars && c == '$' {
            let r = &rest[1..];
            let len = if r.starts_with('{') {
                r.find('}').map_or(r.len(), |k| k + 1)
            } else {
                r.find(|ch: char| !ident_char(ch)).unwrap_or(r.len())
            };
            if len > 0 {
                out.push(base + i..base + i + 1 + len, Tok::Var);
                i += 1 + len;
                continue;
            }
        }
        let prev_ident = i > 0 && line[..i].chars().next_back().is_some_and(ident_char);
        if c.is_ascii_digit() && !prev_ident {
            let len = rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_'))
                .unwrap_or(rest.len());
            out.push(base + i..base + i + len, Tok::Num);
            i += len;
            continue;
        }
        if (ident_char(c) || c == '~') && !prev_ident {
            let len = if c == '~' {
                1
            } else {
                rest.find(|ch: char| !ident_char(ch)).unwrap_or(rest.len())
            };
            let w = &rest[..len];
            let kw = if sp.kw_ci {
                sp.kw.iter().any(|k| k.eq_ignore_ascii_case(w))
            } else {
                sp.kw.contains(&w)
            };
            out.push(
                base + i..base + i + len,
                if kw { Tok::Keyword } else { Tok::Plain },
            );
            i += len;
            continue;
        }
        out.push(base + i..base + i + cl, Tok::Plain);
        i += cl;
    }
    carry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(lang: Lang, text: &str) -> Vec<(String, Tok)> {
        let spans = highlight(lang, text);
        // Spans cover the text exactly, in order.
        let mut at = 0;
        for (r, _) in &spans {
            assert_eq!(r.start, at, "{spans:?}");
            at = r.end;
        }
        assert_eq!(at, text.len());
        spans
            .into_iter()
            .filter(|(_, t)| *t != Tok::Plain)
            .map(|(r, t)| (text[r].to_string(), t))
            .collect()
    }

    fn has(v: &[(String, Tok)], s: &str, t: Tok) -> bool {
        v.iter().any(|(x, k)| x == s && *k == t)
    }

    #[test]
    fn detects_languages_by_name_folder_and_shebang() {
        let d = |p| Lang::detect(p, "");
        assert_eq!(d("/srv/app/docker-compose.yml"), Lang::Yaml);
        assert_eq!(d("/srv/app/Dockerfile"), Lang::Dockerfile);
        assert_eq!(d("/srv/app/.env"), Lang::Env);
        assert_eq!(d("/srv/app/.env.production"), Lang::Env);
        assert_eq!(d("/etc/nginx/nginx.conf"), Lang::Nginx);
        assert_eq!(d("/etc/nginx/sites-available/default"), Lang::Nginx);
        assert_eq!(d("/etc/ssh/sshd_config.d/x.conf"), Lang::Ini);
        assert_eq!(d("C:\\work\\Cargo.toml"), Lang::Toml);
        assert_eq!(d("/home/deploy/.bashrc"), Lang::Shell);
        for (p, l) in [
            ("a.sh", Lang::Shell),
            ("a.json", Lang::Json),
            ("a.ini", Lang::Ini),
            ("a.py", Lang::Python),
            ("a.js", Lang::JavaScript),
            ("a.ts", Lang::TypeScript),
            ("a.php", Lang::Php),
            ("a.sql", Lang::Sql),
            ("README.md", Lang::Markdown),
            ("main.rs", Lang::Rust),
            ("notes.txt", Lang::Plain),
        ] {
            assert_eq!(d(p), l, "{p}");
        }
        assert_eq!(
            Lang::detect("/usr/local/bin/backup", "#!/bin/sh"),
            Lang::Shell
        );
        assert_eq!(Lang::detect("run", "#!/usr/bin/env bash"), Lang::Shell);
        assert_eq!(Lang::detect("tool", "#!/usr/bin/env python3"), Lang::Python);
    }

    #[test]
    fn shell_comments_strings_variables() {
        let v = toks(
            Lang::Shell,
            "if [ -n \"$HOME\" ]; then # check\n  echo a#b ${X} 'it''s'\nfi\n",
        );
        assert!(has(&v, "if", Tok::Keyword));
        assert!(has(&v, "\"$HOME\"", Tok::Str));
        assert!(has(&v, "# check\n", Tok::Comment));
        assert!(
            !v.iter().any(|(s, _)| s.starts_with("#b")),
            "# inside a word is not a comment"
        );
        assert!(has(&v, "${X}", Tok::Var));
        assert!(has(&v, "fi", Tok::Keyword));
    }

    #[test]
    fn yaml_keys_and_values() {
        let v = toks(
            Lang::Yaml,
            "services:\n  web:\n    image: \"nginx:1.27\"\n    ports:\n      - 8080:80 # http\n    restart: no\n",
        );
        assert!(has(&v, "services", Tok::Key));
        assert!(has(&v, "    image", Tok::Key));
        assert!(has(&v, "\"nginx:1.27\"", Tok::Str));
        assert!(has(&v, "# http\n", Tok::Comment));
        assert!(has(&v, "no", Tok::Keyword));
        assert!(
            !has(&v, "      - 8080", Tok::Key),
            "a list value is not a key"
        );
    }

    #[test]
    fn json_keys_versus_strings() {
        let v = toks(
            Lang::Json,
            "{\"name\": \"web\", \"port\": 8080, \"on\": true}",
        );
        assert!(has(&v, "\"name\"", Tok::Key));
        assert!(has(&v, "\"web\"", Tok::Str));
        assert!(has(&v, "8080", Tok::Num));
        assert!(has(&v, "true", Tok::Keyword));
    }

    #[test]
    fn ini_toml_env_sections_and_keys() {
        let v = toks(
            Lang::Ini,
            "[mysqld]\n; comment\nport = 3306\nbind-address: 192.0.2.10\n",
        );
        assert!(has(&v, "[mysqld]", Tok::Heading));
        assert!(has(&v, "; comment\n", Tok::Comment));
        assert!(has(&v, "port ", Tok::Key));
        assert!(has(&v, "bind-address", Tok::Key));
        let v = toks(
            Lang::Toml,
            "[package]\nname = \"telepad\"\ndesc = \"\"\"\nmulti\n\"\"\"\n",
        );
        assert!(has(&v, "[package]", Tok::Heading));
        assert!(has(&v, "\"telepad\"", Tok::Str));
        assert!(has(&v, "\"\"\"\nmulti\n\"\"\"", Tok::Str), "{v:?}");
        let v = toks(
            Lang::Env,
            "# db\nDB_HOST=192.0.2.5\nexport API_URL=\"https://example.test\"\n",
        );
        assert!(has(&v, "DB_HOST", Tok::Key));
        assert!(has(&v, "export API_URL", Tok::Key));
    }

    #[test]
    fn dockerfile_nginx_and_block_comments() {
        let v = toks(
            Lang::Dockerfile,
            "FROM alpine:3.20\nRUN apk add curl # tools\n",
        );
        assert!(has(&v, "FROM", Tok::Keyword) && has(&v, "RUN", Tok::Keyword));
        let v = toks(
            Lang::Nginx,
            "server {\n    listen 80;\n    root $doc_root; # web\n}\n",
        );
        assert!(has(&v, "server", Tok::Key));
        assert!(has(&v, "    listen", Tok::Key));
        assert!(has(&v, "$doc_root", Tok::Var));
        let v = toks(Lang::JavaScript, "/* a\n b */ const x = 'y'; // z\n");
        assert!(has(&v, "/* a\n b */", Tok::Comment), "{v:?}");
        assert!(has(&v, "const", Tok::Keyword));
        assert!(has(&v, "// z\n", Tok::Comment));
    }

    #[test]
    fn python_sql_markdown_rust() {
        let v = toks(
            Lang::Python,
            "def f():\n    \"\"\"doc\n    more\"\"\"\n    return 1\n",
        );
        assert!(has(&v, "def", Tok::Keyword) && has(&v, "return", Tok::Keyword));
        assert!(has(&v, "\"\"\"doc\n    more\"\"\"", Tok::Str), "{v:?}");
        let v = toks(
            Lang::Sql,
            "SELECT id FROM hosts -- all\nwhere name = 'web';",
        );
        assert!(has(&v, "SELECT", Tok::Keyword) && has(&v, "where", Tok::Keyword));
        assert!(has(&v, "'web'", Tok::Str));
        let v = toks(Lang::Markdown, "# Title\ntext `code`\n```\nx = 1\n```\n");
        assert!(has(&v, "# Title\n", Tok::Heading));
        assert!(has(&v, "`code`", Tok::Str));
        assert!(has(&v, "```\nx = 1\n```\n", Tok::Str));
        let v = toks(Lang::Rust, "fn main() { let s = \"a\\\"b\"; }");
        assert!(has(&v, "fn", Tok::Keyword));
        assert!(has(&v, "\"a\\\"b\"", Tok::Str));
        let v = toks(Lang::Php, "<?php $x = 1; // c\n");
        assert!(has(&v, "$x", Tok::Var));
    }

    #[test]
    fn plain_text_and_unicode_never_split_characters() {
        let t = "ünïcode 'x' — ok\n";
        for l in Lang::ALL {
            let _ = toks(l, t);
            let _ = toks(l, "");
            let _ = toks(l, "\"unterminated\n'also");
        }
    }
}
