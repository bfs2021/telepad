//! Pure SQL text helpers: quoting, splitting a script into statements,
//! classifying statements (dangerous / read-only), and the SQL for browsing
//! and editing rows. No I/O; everything here is unit-tested.

use std::ops::Range;

use super::{Column, Engine, Statement, TableRef};

/// `name` quoted as an identifier: `` `a``b` `` (MySQL), `"a""b"` (PostgreSQL).
pub fn quote_ident(engine: Engine, name: &str) -> String {
    match engine {
        Engine::MySql => format!("`{}`", name.replace('`', "``")),
        Engine::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// `s` as a string literal. MySQL also escapes backslashes (default SQL mode).
pub fn quote_str(engine: Engine, s: &str) -> String {
    match engine {
        Engine::MySql => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''")),
        Engine::Postgres => {
            if s.contains('\\') {
                // Standard strings would keep backslashes anyway; E'' makes it explicit.
                format!("E'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
            } else {
                format!("'{}'", s.replace('\'', "''"))
            }
        }
    }
}

/// `` `db`.`table` `` / `"schema"."table"`.
pub fn table_path(engine: Engine, t: &TableRef) -> String {
    match engine {
        Engine::MySql => format!(
            "{}.{}",
            quote_ident(engine, &t.database),
            quote_ident(engine, &t.table)
        ),
        Engine::Postgres => format!(
            "{}.{}",
            quote_ident(engine, &t.schema),
            quote_ident(engine, &t.table)
        ),
    }
}

/// Byte ranges of the statements in `text`, without the `;` and surrounding
/// whitespace. Quotes, comments and PostgreSQL `$tag$` strings are respected.
pub fn split_statements(engine: Engine, text: &str) -> Vec<Range<usize>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let push = |out: &mut Vec<Range<usize>>, s: usize, e: usize| {
        let piece = &text[s..e];
        let lead = piece.len() - piece.trim_start().len();
        let trimmed = piece.trim();
        if !strip_comments(engine, trimmed).trim().is_empty() {
            out.push(s + lead..s + lead + trimmed.len());
        }
    };
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' => i = skip_quoted(engine, b, i),
            b'-' if b.get(i + 1) == Some(&b'-') => i = skip_line(b, i),
            b'#' if engine == Engine::MySql => i = skip_line(b, i),
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = match text[i + 2..].find("*/") {
                    Some(p) => i + 2 + p + 2,
                    None => b.len(),
                }
            }
            b'$' if engine == Engine::Postgres => i = skip_dollar(text, i),
            b';' => {
                push(&mut out, start, i);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    push(&mut out, start, b.len());
    out
}

/// The statement around byte `cursor` (or the one just before it).
pub fn statement_at(engine: Engine, text: &str, cursor: usize) -> Option<Range<usize>> {
    let all = split_statements(engine, text);
    all.iter()
        .find(|r| r.start <= cursor && cursor <= r.end)
        .or_else(|| all.iter().rev().find(|r| r.end <= cursor))
        .or_else(|| all.first())
        .cloned()
}

fn skip_quoted(engine: Engine, b: &[u8], start: usize) -> usize {
    let q = b[start];
    let mut i = start + 1;
    while i < b.len() {
        if b[i] == b'\\' && engine == Engine::MySql && q != b'`' {
            i += 2;
            continue;
        }
        if b[i] == q {
            if b.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

fn skip_line(b: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// `$tag$ … $tag$` (tag may be empty). A lone `$1` parameter is not a quote.
fn skip_dollar(text: &str, start: usize) -> usize {
    let rest = &text[start + 1..];
    let tag_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let tag = &rest[..tag_len];
    if rest.as_bytes().get(tag_len) != Some(&b'$') || tag.starts_with(|c: char| c.is_ascii_digit())
    {
        return start + 1;
    }
    let open = format!("${tag}$");
    let body = start + open.len();
    match text[body..].find(&open) {
        Some(p) => body + p + open.len(),
        None => text.len(),
    }
}

/// Text without comments and with string / identifier contents blanked, so
/// keyword checks do not look inside them.
fn strip_comments(engine: Engine, sql: &str) -> String {
    let b = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' => {
                let end = skip_quoted(engine, b, i);
                out.push_str(" '' ");
                i = end;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => i = skip_line(b, i),
            b'#' if engine == Engine::MySql => i = skip_line(b, i),
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = match sql[i + 2..].find("*/") {
                    Some(p) => i + 2 + p + 2,
                    None => b.len(),
                };
                out.push(' ');
            }
            b'$' if engine == Engine::Postgres => {
                let end = skip_dollar(sql, i);
                if end > i + 1 {
                    out.push_str(" '' ");
                } else {
                    out.push('$');
                }
                i = end;
            }
            _ => {
                let ch = sql[i..].chars().next().unwrap_or(' ');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Upper-case words outside strings and comments.
fn words(engine: Engine, sql: &str) -> Vec<String> {
    strip_comments(engine, sql)
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_uppercase())
        .collect()
}

/// First keyword of a statement (`SELECT`, `UPDATE`…), upper case.
pub fn first_keyword(engine: Engine, sql: &str) -> String {
    words(engine, sql).into_iter().next().unwrap_or_default()
}

/// Why a statement needs a confirmation before it runs, or `None`.
pub fn danger(engine: Engine, sql: &str) -> Option<String> {
    let w = words(engine, sql);
    let first = w.first().map(String::as_str).unwrap_or("");
    let has = |k: &str| w.iter().any(|x| x == k);
    match first {
        "DROP" => Some(format!(
            "DROP {}",
            w.get(1).map(String::as_str).unwrap_or("")
        )),
        "TRUNCATE" => Some("TRUNCATE".into()),
        "DELETE" if !has("WHERE") => Some("DELETE without WHERE (every row)".into()),
        "UPDATE" if !has("WHERE") => Some("UPDATE without WHERE (every row)".into()),
        "ALTER" if has("DROP") => Some("ALTER … DROP".into()),
        _ => None,
    }
}

/// Statements allowed while the connection is switched to Read-only.
pub fn is_read_only(engine: Engine, sql: &str) -> bool {
    matches!(
        first_keyword(engine, sql).as_str(),
        "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "WITH" | "VALUES" | "TABLE" | "USE"
    )
}

/// Sort for the Data tab: column and descending.
pub type Sort = Option<(String, bool)>;

/// `SELECT * FROM t [WHERE …] [ORDER BY …] LIMIT n OFFSET m`. The filter is the
/// user's own SQL condition, as in HeidiSQL.
pub fn select_page(
    engine: Engine,
    t: &TableRef,
    filter: &str,
    sort: &Sort,
    offset: u64,
    limit: u64,
) -> String {
    let mut s = format!("SELECT * FROM {}", table_path(engine, t));
    let f = filter.trim().trim_end_matches(';').trim();
    if !f.is_empty() {
        s.push_str(&format!(" WHERE {f}"));
    }
    if let Some((col, desc)) = sort {
        s.push_str(&format!(
            " ORDER BY {} {}",
            quote_ident(engine, col),
            if *desc { "DESC" } else { "ASC" }
        ));
    }
    s.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
    s
}

/// Placeholder for parameter `n` (1-based) of `col`; PostgreSQL casts text to
/// the column type so every value can be sent as text.
fn param(engine: Engine, n: usize, col: Option<&Column>) -> String {
    match engine {
        Engine::MySql => "?".into(),
        Engine::Postgres => match col.map(|c| c.full_type.trim()).filter(|t| !t.is_empty()) {
            Some(ty) => format!("CAST(${n}::text AS {ty})"),
            None => format!("${n}"),
        },
    }
}

fn find<'a>(columns: &'a [Column], name: &str) -> Option<&'a Column> {
    columns.iter().find(|c| c.name == name)
}

fn key_where(
    engine: Engine,
    columns: &[Column],
    key: &[(String, Option<String>)],
    params: &mut Vec<Option<String>>,
) -> String {
    key.iter()
        .map(|(c, v)| match v {
            None => format!("{} IS NULL", quote_ident(engine, c)),
            Some(v) => {
                params.push(Some(v.clone()));
                format!(
                    "{} = {}",
                    quote_ident(engine, c),
                    param(engine, params.len(), find(columns, c))
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// `UPDATE t SET … WHERE <key>`; one row (the caller checks the row count).
pub fn update_row(
    engine: Engine,
    t: &TableRef,
    columns: &[Column],
    key: &[(String, Option<String>)],
    set: &[(String, Option<String>)],
) -> Statement {
    let mut params = Vec::new();
    let sets = set
        .iter()
        .map(|(c, v)| {
            params.push(v.clone());
            format!(
                "{} = {}",
                quote_ident(engine, c),
                param(engine, params.len(), find(columns, c))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let w = key_where(engine, columns, key, &mut params);
    let mut sql = format!("UPDATE {} SET {sets} WHERE {w}", table_path(engine, t));
    if engine == Engine::MySql {
        sql.push_str(" LIMIT 1");
    }
    Statement { sql, params }
}

/// `INSERT INTO t (…) VALUES (…)`; columns left out get their default.
pub fn insert_row(
    engine: Engine,
    t: &TableRef,
    columns: &[Column],
    values: &[(String, Option<String>)],
) -> Statement {
    let path = table_path(engine, t);
    if values.is_empty() {
        let sql = match engine {
            Engine::MySql => format!("INSERT INTO {path} () VALUES ()"),
            Engine::Postgres => format!("INSERT INTO {path} DEFAULT VALUES"),
        };
        return Statement {
            sql,
            params: vec![],
        };
    }
    let mut params = Vec::new();
    let names: Vec<String> = values.iter().map(|(c, _)| quote_ident(engine, c)).collect();
    let marks: Vec<String> = values
        .iter()
        .map(|(c, v)| {
            params.push(v.clone());
            param(engine, params.len(), find(columns, c))
        })
        .collect();
    Statement {
        sql: format!(
            "INSERT INTO {path} ({}) VALUES ({})",
            names.join(", "),
            marks.join(", ")
        ),
        params,
    }
}

/// `DELETE FROM t WHERE <key>`; one row.
pub fn delete_row(
    engine: Engine,
    t: &TableRef,
    columns: &[Column],
    key: &[(String, Option<String>)],
) -> Statement {
    let mut params = Vec::new();
    let w = key_where(engine, columns, key, &mut params);
    let mut sql = format!("DELETE FROM {} WHERE {w}", table_path(engine, t));
    if engine == Engine::MySql {
        sql.push_str(" LIMIT 1");
    }
    Statement { sql, params }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MY: Engine = Engine::MySql;
    const PG: Engine = Engine::Postgres;

    fn t(db: &str, schema: &str, table: &str) -> TableRef {
        TableRef {
            database: db.into(),
            schema: schema.into(),
            table: table.into(),
        }
    }

    #[test]
    fn quotes_hostile_names_and_values() {
        assert_eq!(quote_ident(MY, "a`b; DROP"), "`a``b; DROP`");
        assert_eq!(quote_ident(PG, "a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_str(MY, "it's \\ ok"), "'it''s \\\\ ok'");
        assert_eq!(quote_str(PG, "it's"), "'it''s'");
        assert_eq!(quote_str(PG, "a\\b"), "E'a\\\\b'");
        assert_eq!(
            table_path(MY, &t("shop", "", "or`ders")),
            "`shop`.`or``ders`"
        );
        assert_eq!(
            table_path(PG, &t("shop", "public", "orders")),
            "\"public\".\"orders\""
        );
    }

    fn pieces(engine: Engine, s: &str) -> Vec<String> {
        split_statements(engine, s)
            .into_iter()
            .map(|r| s[r].to_string())
            .collect()
    }

    #[test]
    fn splits_scripts_respecting_quotes_and_comments() {
        assert_eq!(
            pieces(
                MY,
                "SELECT 1; select ';' -- a;b\n; # x;y\nSELECT `a;b` ;  ;"
            ),
            vec![
                "SELECT 1",
                "select ';' -- a;b",
                "# x;y
SELECT `a;b`"
            ]
        );
        assert_eq!(
            pieces(MY, "INSERT INTO t VALUES ('it\\'s; x'); SELECT 2"),
            vec!["INSERT INTO t VALUES ('it\\'s; x')", "SELECT 2"]
        );
        assert_eq!(
            pieces(
                PG,
                "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql; SELECT $1; /* ; */ SELECT 'a''b;'"
            ),
            vec![
                "CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql",
                "SELECT $1",
                "/* ; */ SELECT 'a''b;'"
            ]
        );
        assert!(pieces(PG, "  -- only a comment\n ; ").is_empty());
    }

    #[test]
    fn finds_the_statement_under_the_cursor() {
        let s = "SELECT 1;\nSELECT 2;\n";
        assert_eq!(&s[statement_at(MY, s, 3).unwrap()], "SELECT 1");
        assert_eq!(&s[statement_at(MY, s, 12).unwrap()], "SELECT 2");
        assert_eq!(&s[statement_at(MY, s, s.len()).unwrap()], "SELECT 2");
        assert_eq!(statement_at(MY, "", 0), None);
    }

    #[test]
    fn flags_dangerous_statements() {
        assert_eq!(danger(MY, "drop table x").as_deref(), Some("DROP TABLE"));
        assert!(danger(MY, "TRUNCATE t").is_some());
        assert!(danger(MY, "DELETE FROM t").is_some());
        assert!(danger(MY, "delete from t where id = 1").is_none());
        assert!(
            danger(MY, "UPDATE t SET a = 'where'").is_some(),
            "WHERE in a string"
        );
        assert!(
            danger(PG, "UPDATE t SET a = 1 -- where\n").is_some(),
            "WHERE in a comment"
        );
        assert!(danger(PG, "update t set a = 1 where b").is_none());
        assert!(danger(MY, "ALTER TABLE t DROP COLUMN c").is_some());
        assert!(danger(MY, "/* drop */ SELECT 1").is_none());
        assert!(danger(MY, "SELECT * FROM t").is_none());
    }

    #[test]
    fn read_only_allows_only_reading_statements() {
        assert!(is_read_only(MY, "  select 1"));
        assert!(is_read_only(PG, "WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(is_read_only(MY, "-- c\nSHOW TABLES"));
        assert!(!is_read_only(MY, "INSERT INTO t VALUES (1)"));
        assert!(!is_read_only(PG, "SET default_transaction_read_only = off"));
        assert!(!is_read_only(MY, "CREATE TABLE x (a int)"));
    }

    #[test]
    fn page_query_with_filter_sort_and_limit() {
        assert_eq!(
            select_page(
                MY,
                &t("shop", "", "orders"),
                " id > 5 ; ",
                &Some(("na`me".into(), true)),
                2000,
                1000
            ),
            "SELECT * FROM `shop`.`orders` WHERE id > 5 ORDER BY `na``me` DESC LIMIT 1000 OFFSET 2000"
        );
        assert_eq!(
            select_page(PG, &t("shop", "public", "orders"), "", &None, 0, 1000),
            "SELECT * FROM \"public\".\"orders\" LIMIT 1000 OFFSET 0"
        );
    }

    fn cols() -> Vec<Column> {
        vec![
            Column {
                name: "id".into(),
                full_type: "integer".into(),
                ..Default::default()
            },
            Column {
                name: "name".into(),
                full_type: "character varying(50)".into(),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn row_statements_bind_values() {
        let tbl = t("shop", "public", "or\"ders");
        let key = vec![("id".to_string(), Some("7".to_string()))];
        let set = vec![("name".to_string(), Some("Bob'; --".to_string()))];

        let u = update_row(MY, &tbl, &cols(), &key, &set);
        assert_eq!(
            u.sql,
            "UPDATE `shop`.`or\"ders` SET `name` = ? WHERE `id` = ? LIMIT 1"
        );
        assert_eq!(u.params, vec![Some("Bob'; --".into()), Some("7".into())]);

        let u = update_row(PG, &tbl, &cols(), &key, &[("name".into(), None)]);
        assert_eq!(
            u.sql,
            "UPDATE \"public\".\"or\"\"ders\" SET \"name\" = CAST($1::text AS character varying(50)) WHERE \"id\" = CAST($2::text AS integer)"
        );
        assert_eq!(u.params, vec![None, Some("7".into())]);

        let i = insert_row(PG, &tbl, &cols(), &set);
        assert_eq!(
            i.sql,
            "INSERT INTO \"public\".\"or\"\"ders\" (\"name\") VALUES (CAST($1::text AS character varying(50)))"
        );
        assert_eq!(
            insert_row(MY, &tbl, &cols(), &[]).sql,
            "INSERT INTO `shop`.`or\"ders` () VALUES ()"
        );
        assert_eq!(
            insert_row(PG, &tbl, &cols(), &[]).sql,
            "INSERT INTO \"public\".\"or\"\"ders\" DEFAULT VALUES"
        );

        let d = delete_row(
            MY,
            &tbl,
            &cols(),
            &[("id".into(), Some("7".into())), ("x".into(), None)],
        );
        assert_eq!(
            d.sql,
            "DELETE FROM `shop`.`or\"ders` WHERE `id` = ? AND `x` IS NULL LIMIT 1"
        );
        assert_eq!(d.params, vec![Some("7".into())]);
    }
}
