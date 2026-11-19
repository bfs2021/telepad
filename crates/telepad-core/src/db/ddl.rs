//! Structure changes (#54): the SQL for the Table tab's editor (columns,
//! indexes, foreign keys), new tables, rename, truncate and drop. Pure text
//! generation; the user sees every statement before it runs.

use std::collections::HashMap;

use super::sql::{quote_ident, quote_str, table_path};
use super::{
    Column, ColumnDefault, Engine, FK_ACTIONS, ForeignKey, Index, IndexKind, TableInfo, TableKind,
    TableRef,
};

/// A column in the editor; `original` is its name before editing (`None` = new).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColumnEdit {
    pub original: Option<String>,
    pub column: Column,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexEdit {
    pub original: Option<String>,
    pub index: Index,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FkEdit {
    pub original: Option<String>,
    pub fk: ForeignKey,
}

/// The structure editor's content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableEdit {
    pub columns: Vec<ColumnEdit>,
    pub indexes: Vec<IndexEdit>,
    pub foreign_keys: Vec<FkEdit>,
}

impl TableEdit {
    pub fn from_info(info: &TableInfo) -> Self {
        Self {
            columns: info
                .columns
                .iter()
                .map(|c| ColumnEdit {
                    original: Some(c.name.clone()),
                    column: c.clone(),
                })
                .collect(),
            indexes: info
                .indexes
                .iter()
                .map(|i| IndexEdit {
                    original: Some(i.name.clone()),
                    index: i.clone(),
                })
                .collect(),
            foreign_keys: info
                .foreign_keys
                .iter()
                .map(|f| FkEdit {
                    original: Some(f.name.clone()),
                    fk: f.clone(),
                })
                .collect(),
        }
    }

    /// A new table starts with an auto-increment `id` primary key.
    pub fn new_table(engine: Engine) -> Self {
        let id = Column {
            name: "id".into(),
            type_name: match engine {
                Engine::MySql => "INT".into(),
                Engine::Postgres => "integer".into(),
            },
            unsigned: engine == Engine::MySql,
            nullable: false,
            auto_increment: true,
            ..Default::default()
        };
        Self {
            columns: vec![ColumnEdit {
                original: None,
                column: id,
            }],
            indexes: vec![IndexEdit {
                original: None,
                index: Index {
                    name: match engine {
                        Engine::MySql => "PRIMARY".into(),
                        Engine::Postgres => String::new(),
                    },
                    kind: IndexKind::Primary,
                    columns: vec!["id".into()],
                    constraint: true,
                },
            }],
            foreign_keys: vec![],
        }
    }

    /// A new, empty column for "Add column".
    pub fn blank_column(engine: Engine, n: usize) -> ColumnEdit {
        ColumnEdit {
            original: None,
            column: Column {
                name: format!("column{n}"),
                type_name: match engine {
                    Engine::MySql => "VARCHAR".into(),
                    Engine::Postgres => "character varying".into(),
                },
                length: "255".into(),
                nullable: true,
                default: match engine {
                    Engine::MySql => ColumnDefault::Null,
                    Engine::Postgres => ColumnDefault::None,
                },
                ..Default::default()
            },
        }
    }
}

/// Write a type in DDL, putting a precision back where PostgreSQL wants it:
/// `timestamp(3) with time zone`.
fn sql_type(c: &Column) -> String {
    let l = c.length.trim();
    let t = c.type_name.trim();
    if !l.is_empty() {
        for prefix in ["timestamp ", "time "] {
            if let Some(rest) = t.strip_prefix(prefix) {
                return format!("{}({l}) {rest}", prefix.trim_end());
            }
        }
    }
    c.sql_type()
}

fn unsafe_sql(s: &str) -> bool {
    s.contains(';') || s.contains("--") || s.contains("/*") || s.contains('\0')
}

/// `255`, `10,2`, `'a','it''s'`: numbers, commas and quoted strings only.
fn valid_length(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'0'..=b'9' | b',' | b' ' => i += 1,
            b'\'' => {
                i += 1;
                loop {
                    match b.get(i) {
                        None => return false,
                        Some(b'\\') => i += 2,
                        Some(b'\'') if b.get(i + 1) == Some(&b'\'') => i += 2,
                        Some(b'\'') => {
                            i += 1;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
            }
            _ => return false,
        }
    }
    true
}

fn valid_type(t: &str) -> bool {
    let t = t.trim();
    !t.is_empty()
        && !unsafe_sql(t)
        && !t.contains('\'')
        && t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '"')
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || " _.\"[](),".contains(c))
}

/// Check one column; the message names the column.
pub fn validate_column(engine: Engine, c: &Column) -> Result<(), String> {
    let n = &c.name;
    if n.trim().is_empty() {
        return Err("A column has no name".into());
    }
    if n.contains('\0') {
        return Err(format!("Column {n:?}: the name contains a NUL character"));
    }
    if !valid_type(&c.type_name) {
        return Err(format!("Column {n}: type {:?} is not valid", c.type_name));
    }
    if !valid_length(c.length.trim()) {
        return Err(format!(
            "Column {n}: length / set {:?} may hold only numbers, commas and quoted values",
            c.length
        ));
    }
    if !c
        .collation
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(format!(
            "Column {n}: collation {:?} is not valid",
            c.collation
        ));
    }
    if let ColumnDefault::Expression(e) = &c.default
        && (e.trim().is_empty() || unsafe_sql(e))
    {
        return Err(format!("Column {n}: default expression {e:?} is not valid"));
    }
    if engine == Engine::Postgres && (c.unsigned || !c.collation.is_empty()) {
        return Err(format!(
            "Column {n}: UNSIGNED and collation are MySQL options"
        ));
    }
    Ok(())
}

fn default_clause(engine: Engine, d: &ColumnDefault) -> String {
    match d {
        ColumnDefault::None => String::new(),
        ColumnDefault::Null => " DEFAULT NULL".into(),
        ColumnDefault::Text(t) => format!(" DEFAULT {}", quote_str(engine, t)),
        ColumnDefault::Expression(e) => format!(" DEFAULT {}", e.trim()),
    }
}

fn default_expr(engine: Engine, d: &ColumnDefault) -> Option<String> {
    match d {
        ColumnDefault::None => None,
        ColumnDefault::Null => Some("NULL".into()),
        ColumnDefault::Text(t) => Some(quote_str(engine, t)),
        ColumnDefault::Expression(e) => Some(e.trim().to_string()),
    }
}

/// Full column definition (MySQL: everything; PostgreSQL: without the comment).
fn column_def(engine: Engine, c: &Column) -> String {
    let q = |s: &str| quote_ident(engine, s);
    match engine {
        Engine::MySql => {
            let mut s = format!("{} {}", q(&c.name), sql_type(c));
            if c.unsigned {
                s.push_str(" UNSIGNED");
            }
            if !c.collation.is_empty() {
                s.push_str(&format!(" COLLATE {}", c.collation));
            }
            s.push_str(if c.nullable { " NULL" } else { " NOT NULL" });
            if !c.auto_increment {
                s.push_str(&default_clause(engine, &c.default));
            } else {
                s.push_str(" AUTO_INCREMENT");
            }
            if !c.comment.is_empty() {
                s.push_str(&format!(" COMMENT {}", quote_str(engine, &c.comment)));
            }
            s
        }
        Engine::Postgres => {
            let mut s = format!("{} {}", q(&c.name), sql_type(c));
            if c.auto_increment {
                s.push_str(" GENERATED BY DEFAULT AS IDENTITY");
            }
            if !c.nullable {
                s.push_str(" NOT NULL");
            }
            if !c.auto_increment {
                s.push_str(&default_clause(engine, &c.default));
            }
            s
        }
    }
}

fn cols(engine: Engine, names: &[String]) -> String {
    names
        .iter()
        .map(|n| quote_ident(engine, n))
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_edit(engine: Engine, edit: &TableEdit) -> Result<(), String> {
    if edit.columns.is_empty() {
        return Err("A table needs at least one column".into());
    }
    let mut seen: Vec<&str> = Vec::new();
    for c in &edit.columns {
        validate_column(engine, &c.column)?;
        if seen.contains(&c.column.name.as_str()) {
            return Err(format!("Two columns are named {}", c.column.name));
        }
        seen.push(&c.column.name);
    }
    if edit
        .indexes
        .iter()
        .filter(|i| i.index.kind == IndexKind::Primary)
        .count()
        > 1
    {
        return Err("A table can have only one PRIMARY key".into());
    }
    for i in &edit.indexes {
        let ix = &i.index;
        if ix.columns.is_empty() {
            return Err(format!("Index {}: choose at least one column", ix.name));
        }
        if let Some(c) = ix.columns.iter().find(|c| !seen.contains(&c.as_str())) {
            return Err(format!("Index {}: there is no column {c}", ix.name));
        }
        if ix.kind != IndexKind::Primary && ix.name.trim().is_empty() {
            return Err("An index has no name".into());
        }
        if ix.kind == IndexKind::Fulltext && engine == Engine::Postgres {
            return Err("FULLTEXT indexes are MySQL only".into());
        }
    }
    for f in &edit.foreign_keys {
        let fk = &f.fk;
        if fk.name.trim().is_empty() {
            return Err("A foreign key has no name".into());
        }
        if fk.columns.is_empty() || fk.columns.len() != fk.ref_columns.len() {
            return Err(format!(
                "Foreign key {}: choose the same number of columns on both sides",
                fk.name
            ));
        }
        if let Some(c) = fk.columns.iter().find(|c| !seen.contains(&c.as_str())) {
            return Err(format!("Foreign key {}: there is no column {c}", fk.name));
        }
        if fk.ref_table.trim().is_empty() {
            return Err(format!(
                "Foreign key {}: choose the referenced table",
                fk.name
            ));
        }
        for a in [&fk.on_update, &fk.on_delete] {
            if !a.is_empty() && !FK_ACTIONS.contains(&a.as_str()) {
                return Err(format!("Foreign key {}: unknown action {a}", fk.name));
            }
        }
    }
    Ok(())
}

fn fk_clause(engine: Engine, fk: &ForeignKey, own_schema: &str) -> String {
    let schema = if fk.ref_schema.is_empty() {
        own_schema
    } else {
        &fk.ref_schema
    };
    let mut s = format!(
        "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}.{} ({})",
        quote_ident(engine, &fk.name),
        cols(engine, &fk.columns),
        quote_ident(engine, schema),
        quote_ident(engine, &fk.ref_table),
        cols(engine, &fk.ref_columns)
    );
    if !fk.on_delete.is_empty() {
        s.push_str(&format!(" ON DELETE {}", fk.on_delete));
    }
    if !fk.on_update.is_empty() {
        s.push_str(&format!(" ON UPDATE {}", fk.on_update));
    }
    s
}

fn own_schema(t: &TableRef, engine: Engine) -> &str {
    match engine {
        Engine::MySql => &t.database,
        Engine::Postgres => &t.schema,
    }
}

/// `CREATE TABLE` (+ PostgreSQL indexes and comments) for a new table `t`.
pub fn create(engine: Engine, t: &TableRef, edit: &TableEdit) -> Result<Vec<String>, String> {
    if t.table.trim().is_empty() {
        return Err("Enter a name for the new table".into());
    }
    check_edit(engine, edit)?;
    let mut parts: Vec<String> = edit
        .columns
        .iter()
        .map(|c| column_def(engine, &c.column))
        .collect();
    let mut after = Vec::new();
    let path = table_path(engine, t);
    for i in &edit.indexes {
        let ix = &i.index;
        let c = cols(engine, &ix.columns);
        let name = quote_ident(engine, &ix.name);
        match (engine, ix.kind) {
            (_, IndexKind::Primary) => parts.push(format!("PRIMARY KEY ({c})")),
            (Engine::MySql, IndexKind::Unique) => parts.push(format!("UNIQUE KEY {name} ({c})")),
            (Engine::MySql, IndexKind::Key) => parts.push(format!("KEY {name} ({c})")),
            (Engine::MySql, IndexKind::Fulltext) => {
                parts.push(format!("FULLTEXT KEY {name} ({c})"))
            }
            (Engine::Postgres, IndexKind::Unique) => {
                parts.push(format!("CONSTRAINT {name} UNIQUE ({c})"))
            }
            (Engine::Postgres, _) => after.push(format!("CREATE INDEX {name} ON {path} ({c})")),
        }
    }
    for f in &edit.foreign_keys {
        parts.push(fk_clause(engine, &f.fk, own_schema(t, engine)));
    }
    let mut out = vec![format!(
        "CREATE TABLE {path} (\n  {}\n)",
        parts.join(",\n  ")
    )];
    out.extend(after);
    if engine == Engine::Postgres {
        out.extend(
            edit.columns
                .iter()
                .filter(|c| !c.column.comment.is_empty())
                .map(|c| {
                    format!(
                        "COMMENT ON COLUMN {path}.{} IS {}",
                        quote_ident(engine, &c.column.name),
                        quote_str(engine, &c.column.comment)
                    )
                }),
        );
    }
    Ok(out)
}

fn same_column(a: &Column, b: &Column) -> bool {
    a.name == b.name
        && a.type_name.eq_ignore_ascii_case(&b.type_name)
        && a.length.trim() == b.length.trim()
        && a.unsigned == b.unsigned
        && a.nullable == b.nullable
        && (a.default == b.default || (a.auto_increment && b.auto_increment))
        && a.auto_increment == b.auto_increment
        && a.comment == b.comment
        && a.collation == b.collation
}

/// Index and key columns still naming a renamed column get the new name.
fn follow_renames(edit: &TableEdit) -> TableEdit {
    let names: Vec<&str> = edit
        .columns
        .iter()
        .map(|c| c.column.name.as_str())
        .collect();
    let renames: HashMap<String, String> = edit
        .columns
        .iter()
        .filter_map(|c| {
            let o = c.original.as_deref()?;
            (o != c.column.name && !names.contains(&o))
                .then(|| (o.to_string(), c.column.name.clone()))
        })
        .collect();
    let map = |v: &mut Vec<String>| {
        for c in v.iter_mut() {
            if let Some(n) = renames.get(c) {
                *c = n.clone();
            }
        }
    };
    let mut e = edit.clone();
    e.indexes.iter_mut().for_each(|i| map(&mut i.index.columns));
    e.foreign_keys
        .iter_mut()
        .for_each(|f| map(&mut f.fk.columns));
    e
}

/// Statements that turn `before` into `edit`; empty when nothing changed.
pub fn alter(engine: Engine, before: &TableInfo, edit: &TableEdit) -> Result<Vec<String>, String> {
    if before.kind == TableKind::View {
        return Err("Views cannot be changed here; use the Query tab".into());
    }
    let edit = &follow_renames(edit);
    check_edit(engine, edit)?;
    let t = &before.table;
    let path = table_path(engine, t);
    let q = |s: &str| quote_ident(engine, s);

    // Old name -> new name, so index and key columns compare after a rename.
    let renames: HashMap<&str, &str> = edit
        .columns
        .iter()
        .filter_map(|c| c.original.as_deref().map(|o| (o, c.column.name.as_str())))
        .collect();
    let map_cols = |v: &[String]| -> Vec<String> {
        v.iter()
            .map(|c| renames.get(c.as_str()).map_or(c.clone(), |n| n.to_string()))
            .collect()
    };
    let kept: Vec<&str> = edit
        .columns
        .iter()
        .filter_map(|c| c.original.as_deref())
        .collect();
    let dropped: Vec<&Column> = before
        .columns
        .iter()
        .filter(|c| !kept.contains(&c.name.as_str()))
        .collect();
    // Column order among the columns that stay.
    let before_order: Vec<&str> = before
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .filter(|n| kept.contains(n))
        .collect();
    let moved = |orig: &str| {
        let pos_edit = kept.iter().position(|n| *n == orig);
        let pos_before = before_order.iter().position(|n| *n == orig);
        let prev_edit = pos_edit.and_then(|p| p.checked_sub(1)).map(|p| kept[p]);
        let prev_before = pos_before
            .and_then(|p| p.checked_sub(1))
            .map(|p| before_order[p]);
        prev_edit != prev_before
    };

    let index_changed = |e: &IndexEdit| -> bool {
        match e
            .original
            .as_deref()
            .and_then(|o| before.indexes.iter().find(|i| i.name == o))
        {
            None => true,
            Some(b) => {
                b.kind != e.index.kind
                    || map_cols(&b.columns) != map_cols(&e.index.columns)
                    || (b.name != e.index.name && b.kind != IndexKind::Primary)
            }
        }
    };
    let drop_indexes: Vec<&Index> = before
        .indexes
        .iter()
        .filter(|b| {
            match edit
                .indexes
                .iter()
                .find(|e| e.original.as_deref() == Some(&b.name))
            {
                None => true,
                Some(e) => index_changed(e),
            }
        })
        .collect();
    let add_indexes: Vec<&Index> = edit
        .indexes
        .iter()
        .filter(|e| index_changed(e))
        .map(|e| &e.index)
        .collect();

    let fk_changed = |e: &FkEdit| -> bool {
        match e
            .original
            .as_deref()
            .and_then(|o| before.foreign_keys.iter().find(|f| f.name == o))
        {
            None => true,
            Some(b) => {
                let mut b2 = b.clone();
                b2.columns = map_cols(&b.columns);
                let mut e2 = e.fk.clone();
                e2.columns = map_cols(&e.fk.columns);
                b2 != e2
            }
        }
    };
    let drop_fks: Vec<&ForeignKey> = before
        .foreign_keys
        .iter()
        .filter(|b| {
            match edit
                .foreign_keys
                .iter()
                .find(|e| e.original.as_deref() == Some(&b.name))
            {
                None => true,
                Some(e) => fk_changed(e),
            }
        })
        .collect();
    let add_fks: Vec<&ForeignKey> = edit
        .foreign_keys
        .iter()
        .filter(|e| fk_changed(e))
        .map(|e| &e.fk)
        .collect();

    let mut out = Vec::new();
    match engine {
        Engine::MySql => {
            if !drop_fks.is_empty() {
                let parts: Vec<String> = drop_fks
                    .iter()
                    .map(|f| format!("DROP FOREIGN KEY {}", q(&f.name)))
                    .collect();
                out.push(format!("ALTER TABLE {path}\n  {}", parts.join(",\n  ")));
            }
            let mut parts: Vec<String> = Vec::new();
            for i in &drop_indexes {
                parts.push(if i.kind == IndexKind::Primary {
                    "DROP PRIMARY KEY".into()
                } else {
                    format!("DROP INDEX {}", q(&i.name))
                });
            }
            for c in &dropped {
                parts.push(format!("DROP COLUMN {}", q(&c.name)));
            }
            for (i, e) in edit.columns.iter().enumerate() {
                let pos = match i.checked_sub(1) {
                    None => " FIRST".to_string(),
                    Some(p) => format!(" AFTER {}", q(&edit.columns[p].column.name)),
                };
                match &e.original {
                    None => {
                        parts.push(format!("ADD COLUMN {}{pos}", column_def(engine, &e.column)))
                    }
                    Some(o) => {
                        let Some(b) = before.columns.iter().find(|c| &c.name == o) else {
                            return Err(format!("Column {o} is not in the table any more; reload"));
                        };
                        let mv = moved(o);
                        if !same_column(b, &e.column) || mv {
                            parts.push(format!(
                                "CHANGE COLUMN {} {}{}",
                                q(o),
                                column_def(engine, &e.column),
                                if mv { pos } else { String::new() }
                            ));
                        }
                    }
                }
            }
            for i in &add_indexes {
                let c = cols(engine, &i.columns);
                parts.push(match i.kind {
                    IndexKind::Primary => format!("ADD PRIMARY KEY ({c})"),
                    IndexKind::Unique => format!("ADD UNIQUE INDEX {} ({c})", q(&i.name)),
                    IndexKind::Key => format!("ADD INDEX {} ({c})", q(&i.name)),
                    IndexKind::Fulltext => format!("ADD FULLTEXT INDEX {} ({c})", q(&i.name)),
                });
            }
            if !parts.is_empty() {
                out.push(format!("ALTER TABLE {path}\n  {}", parts.join(",\n  ")));
            }
            if !add_fks.is_empty() {
                let parts: Vec<String> = add_fks
                    .iter()
                    .map(|f| format!("ADD {}", fk_clause(engine, f, &t.database)))
                    .collect();
                out.push(format!("ALTER TABLE {path}\n  {}", parts.join(",\n  ")));
            }
        }
        Engine::Postgres => {
            if kept.iter().any(|o| moved(o)) {
                return Err("PostgreSQL cannot reorder columns; keep their order".into());
            }
            for f in &drop_fks {
                out.push(format!("ALTER TABLE {path} DROP CONSTRAINT {}", q(&f.name)));
            }
            for i in &drop_indexes {
                out.push(if i.constraint || i.kind == IndexKind::Primary {
                    format!("ALTER TABLE {path} DROP CONSTRAINT {}", q(&i.name))
                } else {
                    format!("DROP INDEX {}.{}", q(&t.schema), q(&i.name))
                });
            }
            for c in &dropped {
                out.push(format!("ALTER TABLE {path} DROP COLUMN {}", q(&c.name)));
            }
            for e in &edit.columns {
                let c = &e.column;
                let Some(o) = &e.original else {
                    out.push(format!(
                        "ALTER TABLE {path} ADD COLUMN {}",
                        column_def(engine, c)
                    ));
                    if !c.comment.is_empty() {
                        out.push(format!(
                            "COMMENT ON COLUMN {path}.{} IS {}",
                            q(&c.name),
                            quote_str(engine, &c.comment)
                        ));
                    }
                    continue;
                };
                let Some(b) = before.columns.iter().find(|x| &x.name == o) else {
                    return Err(format!("Column {o} is not in the table any more; reload"));
                };
                if b.name != c.name {
                    out.push(format!(
                        "ALTER TABLE {path} RENAME COLUMN {} TO {}",
                        q(o),
                        q(&c.name)
                    ));
                }
                let col = format!("ALTER TABLE {path} ALTER COLUMN {}", q(&c.name));
                let ty = sql_type(c);
                if sql_type(b) != ty {
                    out.push(format!("{col} TYPE {ty} USING {}::{ty}", q(&c.name)));
                }
                if b.nullable != c.nullable {
                    out.push(format!(
                        "{col} {} NOT NULL",
                        if c.nullable { "DROP" } else { "SET" }
                    ));
                }
                if b.auto_increment && !c.auto_increment {
                    out.push(format!("{col} DROP IDENTITY IF EXISTS"));
                }
                if !c.auto_increment && b.default != c.default {
                    out.push(match default_expr(engine, &c.default) {
                        Some(d) => format!("{col} SET DEFAULT {d}"),
                        None => format!("{col} DROP DEFAULT"),
                    });
                }
                if !b.auto_increment && c.auto_increment {
                    if b.default != ColumnDefault::None {
                        out.push(format!("{col} DROP DEFAULT"));
                    }
                    out.push(format!("{col} ADD GENERATED BY DEFAULT AS IDENTITY"));
                }
                if b.comment != c.comment {
                    out.push(format!(
                        "COMMENT ON COLUMN {path}.{} IS {}",
                        q(&c.name),
                        if c.comment.is_empty() {
                            "NULL".to_string()
                        } else {
                            quote_str(engine, &c.comment)
                        }
                    ));
                }
            }
            for i in &add_indexes {
                let c = cols(engine, &i.columns);
                let named = |kw: &str| {
                    if i.name.trim().is_empty() {
                        format!("ALTER TABLE {path} ADD {kw} ({c})")
                    } else {
                        format!(
                            "ALTER TABLE {path} ADD CONSTRAINT {} {kw} ({c})",
                            q(&i.name)
                        )
                    }
                };
                out.push(match i.kind {
                    IndexKind::Primary => named("PRIMARY KEY"),
                    IndexKind::Unique => named("UNIQUE"),
                    _ => format!("CREATE INDEX {} ON {path} ({c})", q(&i.name)),
                });
            }
            for f in &add_fks {
                out.push(format!(
                    "ALTER TABLE {path} ADD {}",
                    fk_clause(engine, f, &t.schema)
                ));
            }
        }
    }
    Ok(out)
}

/// Rename a table or view (same database / schema).
pub fn rename(engine: Engine, t: &TableRef, new_name: &str) -> Result<String, String> {
    let n = new_name.trim();
    if n.is_empty() || n.contains('\0') {
        return Err("Enter the new name".into());
    }
    let to = TableRef {
        table: n.to_string(),
        ..t.clone()
    };
    Ok(match engine {
        Engine::MySql => format!(
            "RENAME TABLE {} TO {}",
            table_path(engine, t),
            table_path(engine, &to)
        ),
        Engine::Postgres => format!(
            "ALTER TABLE {} RENAME TO {}",
            table_path(engine, t),
            quote_ident(engine, n)
        ),
    })
}

pub fn truncate(engine: Engine, t: &TableRef) -> String {
    format!("TRUNCATE TABLE {}", table_path(engine, t))
}

/// `what` is the tree entry's kind text (`view`, `materialized view`, `InnoDB`…).
pub fn drop(engine: Engine, t: &TableRef, kind: TableKind, what: &str) -> String {
    let object = match kind {
        TableKind::Table => "TABLE",
        TableKind::View if engine == Engine::Postgres && what == "materialized view" => {
            "MATERIALIZED VIEW"
        }
        TableKind::View => "VIEW",
    };
    format!("DROP {object} {}", table_path(engine, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MY: Engine = Engine::MySql;
    const PG: Engine = Engine::Postgres;

    fn t(engine: Engine) -> TableRef {
        TableRef {
            database: "shop".into(),
            schema: if engine == PG {
                "public".into()
            } else {
                String::new()
            },
            table: "items".into(),
        }
    }

    fn col(name: &str, ty: &str, len: &str, nullable: bool) -> Column {
        Column {
            name: name.into(),
            type_name: ty.into(),
            length: len.into(),
            nullable,
            ..Default::default()
        }
    }

    fn info(engine: Engine) -> TableInfo {
        let mut id = col(
            "id",
            if engine == MY { "INT" } else { "integer" },
            "",
            false,
        );
        id.auto_increment = true;
        TableInfo {
            table: t(engine),
            kind: TableKind::Table,
            columns: vec![
                id,
                col(
                    "name",
                    if engine == MY {
                        "VARCHAR"
                    } else {
                        "character varying"
                    },
                    "50",
                    false,
                ),
                col("note", "text", "", true),
            ],
            indexes: vec![
                Index {
                    name: if engine == MY {
                        "PRIMARY".into()
                    } else {
                        "items_pkey".into()
                    },
                    kind: IndexKind::Primary,
                    columns: vec!["id".into()],
                    constraint: true,
                },
                Index {
                    name: "u_name".into(),
                    kind: IndexKind::Unique,
                    columns: vec!["name".into()],
                    constraint: engine == PG,
                },
            ],
            foreign_keys: vec![],
            create_sql: String::new(),
        }
    }

    #[test]
    fn unchanged_structure_needs_no_statement() {
        for e in [MY, PG] {
            let i = info(e);
            assert_eq!(
                alter(e, &i, &TableEdit::from_info(&i)).unwrap(),
                Vec::<String>::new()
            );
        }
    }

    #[test]
    fn mysql_alter_renames_changes_adds_drops_and_moves() {
        let i = info(MY);
        let mut e = TableEdit::from_info(&i);
        e.columns[1].column.name = "title".into(); // rename name -> title (unique index follows)
        e.columns[1].column.length = "80".into();
        e.columns[1].column.comment = "it's shown".into();
        e.columns.remove(2); // drop note
        let mut price = col("price", "DECIMAL", "10,2", true);
        price.unsigned = true;
        price.default = ColumnDefault::Text("0".into());
        e.columns.insert(
            1,
            ColumnEdit {
                original: None,
                column: price,
            },
        );
        e.indexes.push(IndexEdit {
            original: None,
            index: Index {
                name: "k_price".into(),
                kind: IndexKind::Key,
                columns: vec!["price".into()],
                constraint: false,
            },
        });
        let sql = alter(MY, &i, &e).unwrap();
        assert_eq!(
            sql,
            vec![
                "ALTER TABLE `shop`.`items`\n  DROP COLUMN `note`,\n  ADD COLUMN `price` DECIMAL(10,2) UNSIGNED NULL DEFAULT '0' AFTER `id`,\n  \
                 CHANGE COLUMN `name` `title` VARCHAR(80) NOT NULL COMMENT 'it''s shown',\n  ADD INDEX `k_price` (`price`)"
                    .to_string()
            ]
        );

        // Move note first.
        let mut e = TableEdit::from_info(&i);
        let note = e.columns.remove(2);
        e.columns.insert(0, note);
        assert_eq!(
            alter(MY, &i, &e).unwrap(),
            vec!["ALTER TABLE `shop`.`items`\n  CHANGE COLUMN `note` `note` text NULL FIRST,\n  CHANGE COLUMN `id` `id` INT NOT NULL AUTO_INCREMENT AFTER `note`".to_string()]
        );
    }

    #[test]
    fn mysql_index_and_foreign_key_changes() {
        let mut i = info(MY);
        i.foreign_keys.push(ForeignKey {
            name: "fk_cat".into(),
            columns: vec!["name".into()],
            ref_schema: "shop".into(),
            ref_table: "cats".into(),
            ref_columns: vec!["code".into()],
            on_update: "RESTRICT".into(),
            on_delete: "CASCADE".into(),
        });
        let mut e = TableEdit::from_info(&i);
        e.indexes[1].index.kind = IndexKind::Key; // unique -> plain: drop + add
        e.foreign_keys[0].fk.on_delete = "SET NULL".into();
        let sql = alter(MY, &i, &e).unwrap();
        assert_eq!(
            sql[0],
            "ALTER TABLE `shop`.`items`\n  DROP FOREIGN KEY `fk_cat`"
        );
        assert_eq!(
            sql[1],
            "ALTER TABLE `shop`.`items`\n  DROP INDEX `u_name`,\n  ADD INDEX `u_name` (`name`)"
        );
        assert_eq!(
            sql[2],
            "ALTER TABLE `shop`.`items`\n  ADD CONSTRAINT `fk_cat` FOREIGN KEY (`name`) REFERENCES `shop`.`cats` (`code`) ON DELETE SET NULL ON UPDATE RESTRICT"
        );
    }

    #[test]
    fn postgres_alter_uses_separate_statements() {
        let i = info(PG);
        let mut e = TableEdit::from_info(&i);
        e.columns[1].column.name = "title".into();
        e.columns[1].column.length = "80".into();
        e.columns[2].column.nullable = false;
        e.columns[2].column.default = ColumnDefault::Text("none".into());
        e.columns[2].column.comment = "a note".into();
        e.indexes.remove(1); // drop the unique constraint
        e.columns.push(ColumnEdit {
            original: None,
            column: col("price", "numeric", "10,2", true),
        });
        let sql = alter(PG, &i, &e).unwrap();
        assert_eq!(
            sql,
            vec![
                "ALTER TABLE \"public\".\"items\" DROP CONSTRAINT \"u_name\"",
                "ALTER TABLE \"public\".\"items\" RENAME COLUMN \"name\" TO \"title\"",
                "ALTER TABLE \"public\".\"items\" ALTER COLUMN \"title\" TYPE character varying(80) USING \"title\"::character varying(80)",
                "ALTER TABLE \"public\".\"items\" ALTER COLUMN \"note\" SET NOT NULL",
                "ALTER TABLE \"public\".\"items\" ALTER COLUMN \"note\" SET DEFAULT 'none'",
                "COMMENT ON COLUMN \"public\".\"items\".\"note\" IS 'a note'",
                "ALTER TABLE \"public\".\"items\" ADD COLUMN \"price\" numeric(10,2)",
            ]
        );
        let mut moved = TableEdit::from_info(&i);
        moved.columns.swap(0, 1);
        assert!(alter(PG, &i, &moved).unwrap_err().contains("reorder"));
    }

    #[test]
    fn create_tables() {
        let mut e = TableEdit::new_table(MY);
        e.columns.push(TableEdit::blank_column(MY, 2));
        e.indexes.push(IndexEdit {
            original: None,
            index: Index {
                name: "k_c".into(),
                kind: IndexKind::Key,
                columns: vec!["column2".into()],
                constraint: false,
            },
        });
        let mut nt = t(MY);
        nt.table = "new`one".into();
        assert_eq!(
            create(MY, &nt, &e).unwrap(),
            vec!["CREATE TABLE `shop`.`new``one` (\n  `id` INT UNSIGNED NOT NULL AUTO_INCREMENT,\n  `column2` VARCHAR(255) NULL DEFAULT NULL,\n  PRIMARY KEY (`id`),\n  KEY `k_c` (`column2`)\n)".to_string()]
        );
        let mut e = TableEdit::new_table(PG);
        let mut c = TableEdit::blank_column(PG, 2);
        c.column.comment = "made up".into();
        e.columns.push(c);
        e.indexes.push(IndexEdit {
            original: None,
            index: Index {
                name: "k_c".into(),
                kind: IndexKind::Key,
                columns: vec!["column2".into()],
                constraint: false,
            },
        });
        assert_eq!(
            create(PG, &t(PG), &e).unwrap(),
            vec![
                "CREATE TABLE \"public\".\"items\" (\n  \"id\" integer GENERATED BY DEFAULT AS IDENTITY NOT NULL,\n  \"column2\" character varying(255),\n  PRIMARY KEY (\"id\")\n)",
                "CREATE INDEX \"k_c\" ON \"public\".\"items\" (\"column2\")",
                "COMMENT ON COLUMN \"public\".\"items\".\"column2\" IS 'made up'",
            ]
        );
        let mut nt = t(MY);
        nt.table = " ".into();
        assert!(create(MY, &nt, &TableEdit::new_table(MY)).is_err());
    }

    #[test]
    fn rejects_unsafe_or_broken_definitions() {
        let i = info(MY);
        let mut e = TableEdit::from_info(&i);
        e.columns[1].column.type_name = "INT; DROP TABLE x".into();
        assert!(alter(MY, &i, &e).unwrap_err().contains("type"));
        let mut e = TableEdit::from_info(&i);
        e.columns[1].column.length = "1) ; DROP TABLE x; --".into();
        assert!(alter(MY, &i, &e).is_err());
        let mut e = TableEdit::from_info(&i);
        e.columns[1].column.length = "'a','it''s','b\\'c'".into();
        e.columns[1].column.type_name = "ENUM".into();
        assert!(alter(MY, &i, &e).is_ok());
        let mut e = TableEdit::from_info(&i);
        e.columns[2].column.name = "name".into();
        assert!(alter(MY, &i, &e).unwrap_err().contains("Two columns"));
        let mut e = TableEdit::from_info(&i);
        e.indexes[1].index.columns = vec!["missing".into()];
        assert!(alter(MY, &i, &e).unwrap_err().contains("no column"));
        let mut e = TableEdit::from_info(&i);
        e.columns[2].column.default = ColumnDefault::Expression("1; DROP TABLE x".into());
        assert!(alter(MY, &i, &e).is_err());
        let mut v = info(MY);
        v.kind = TableKind::View;
        assert!(alter(MY, &v, &TableEdit::from_info(&v)).is_err());
    }

    #[test]
    fn rename_truncate_drop() {
        assert_eq!(
            rename(MY, &t(MY), "goods").unwrap(),
            "RENAME TABLE `shop`.`items` TO `shop`.`goods`"
        );
        assert_eq!(
            rename(PG, &t(PG), "go\"ods").unwrap(),
            "ALTER TABLE \"public\".\"items\" RENAME TO \"go\"\"ods\""
        );
        assert!(rename(PG, &t(PG), "  ").is_err());
        assert_eq!(truncate(MY, &t(MY)), "TRUNCATE TABLE `shop`.`items`");
        assert_eq!(
            drop(PG, &t(PG), TableKind::View, "materialized view"),
            "DROP MATERIALIZED VIEW \"public\".\"items\""
        );
        assert_eq!(
            drop(MY, &t(MY), TableKind::Table, "InnoDB"),
            "DROP TABLE `shop`.`items`"
        );
    }
}
