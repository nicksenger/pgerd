//! Reads a PostgreSQL schema (tables, columns, primary keys and foreign keys)
//! from an information-schema / pg_catalog query.

use std::collections::HashMap;

use postgres::{Client, NoTls};

/// A single column of a table.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub is_primary_key: bool,
    pub is_foreign_key: bool,
    /// True when this foreign key references the column's own table.
    pub is_self_foreign_key: bool,
}

/// A base table (relation).
#[derive(Debug, Clone)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub columns: Vec<Column>,
}

impl Table {
    /// Fully-qualified identifier used as the node key.
    pub fn key(&self) -> (String, String) {
        (self.schema.clone(), self.name.clone())
    }
}

/// A foreign key constraint: `from_table.from_columns` references
/// `to_table`.
#[derive(Debug, Clone)]
pub struct ForeignKey {
    pub from_schema: String,
    pub from_table: String,
    pub to_schema: String,
    pub to_table: String,
    pub from_columns: Vec<String>,
}

impl ForeignKey {
    /// True when the constraint references its own table.
    pub fn is_self(&self) -> bool {
        self.from_schema == self.to_schema && self.from_table == self.to_table
    }
}

/// The full schema model consumed by the ERD renderer.
#[derive(Debug, Clone)]
pub struct Schema {
    pub tables: Vec<Table>,
    pub foreign_keys: Vec<ForeignKey>,
}

/// Connect to `url` and load the schema of all base tables in non-system
/// schemas.
pub fn load(url: &str) -> Result<Schema, String> {
    let mut client = Client::connect(url, NoTls).map_err(|e| e.to_string())?;

    // Columns (in declaration order) for every base table.
    let column_sql = "
        SELECT c.table_schema, c.table_name, c.column_name, c.data_type
        FROM information_schema.columns c
        JOIN information_schema.tables t
          ON t.table_schema = c.table_schema
         AND t.table_name = c.table_name
        WHERE t.table_type = 'BASE TABLE'
          AND c.table_schema NOT IN ('pg_catalog', 'information_schema')
        ORDER BY c.table_schema, c.table_name, c.ordinal_position";

    let pk_sql = "
        SELECT kcu.table_schema, kcu.table_name, kcu.column_name
        FROM information_schema.table_constraints tc
        JOIN information_schema.key_column_usage kcu
          ON tc.constraint_name = kcu.constraint_name
         AND tc.table_schema = kcu.table_schema
         AND tc.table_name = kcu.table_name
        WHERE tc.constraint_type = 'PRIMARY KEY'";

    let fk_sql = "
        SELECT
            srcns.nspname AS from_schema,
            src.relname   AS from_table,
            tgtns.nspname AS to_schema,
            tgt.relname   AS to_table,
            (SELECT array_agg(att.attname ORDER BY u.ord)
               FROM unnest(con.conkey) WITH ORDINALITY AS u(attnum, ord)
               JOIN pg_attribute att
                 ON att.attrelid = con.conrelid AND att.attnum = u.attnum) AS from_columns
        FROM pg_constraint con
        JOIN pg_class src      ON src.oid = con.conrelid
        JOIN pg_namespace srcns ON srcns.oid = src.relnamespace
        JOIN pg_class tgt      ON tgt.oid = con.confrelid
        JOIN pg_namespace tgtns ON tgtns.oid = tgt.relnamespace
        WHERE con.contype = 'f'
          AND srcns.nspname NOT IN ('pg_catalog', 'information_schema')";

    // --- columns ---------------------------------------------------------
    let column_rows = client.query(column_sql, &[]).map_err(|e| e.to_string())?;
    let mut tables: Vec<Table> = Vec::new();
    let mut table_index: HashMap<(String, String), usize> = HashMap::new();

    for row in column_rows {
        let schema: String = row.get(0);
        let name: String = row.get(1);
        let column_name: String = row.get(2);
        let data_type: String = row.get(3);

        let key = (schema.clone(), name.clone());
        let position = match table_index.get(&key).copied() {
            Some(position) => position,
            None => {
                tables.push(Table {
                    schema: schema.clone(),
                    name: name.clone(),
                    columns: Vec::new(),
                });
                let position = tables.len() - 1;
                table_index.insert(key, position);
                position
            }
        };
        tables[position].columns.push(Column {
            name: column_name,
            data_type,
            is_primary_key: false,
            is_foreign_key: false,
            is_self_foreign_key: false,
        });
    }

    // --- primary keys ----------------------------------------------------
    let pk_rows = client.query(pk_sql, &[]).map_err(|e| e.to_string())?;
    for row in pk_rows {
        let schema: String = row.get(0);
        let name: String = row.get(1);
        let column_name: String = row.get(2);
        if let Some(position) = table_index.get(&(schema.clone(), name)) {
            mark_column(&mut tables[*position], &column_name, |c| {
                c.is_primary_key = true
            });
        }
    }

    // --- foreign keys ----------------------------------------------------
    let fk_rows = client.query(fk_sql, &[]).map_err(|e| e.to_string())?;
    let mut foreign_keys: Vec<ForeignKey> = Vec::new();
    for row in fk_rows {
        let from_schema: String = row.get(0);
        let from_table: String = row.get(1);
        let to_schema: String = row.get(2);
        let to_table: String = row.get(3);
        let from_columns: Vec<String> = row.get::<_, Option<Vec<String>>>(4).unwrap_or_default();

        let is_self_ref = from_schema == to_schema && from_table == to_table;
        if let Some(position) = table_index.get(&(from_schema.clone(), from_table.clone())) {
            for column in &from_columns {
                mark_column(&mut tables[*position], column, |c| {
                    c.is_foreign_key = true;
                    if is_self_ref {
                        c.is_self_foreign_key = true;
                    }
                });
            }
        }

        foreign_keys.push(ForeignKey {
            from_schema,
            from_table,
            to_schema,
            to_table,
            from_columns,
        });
    }

    Ok(Schema {
        tables,
        foreign_keys,
    })
}

fn mark_column(table: &mut Table, column_name: &str, set: impl Fn(&mut Column)) {
    for column in table.columns.iter_mut() {
        if column.name == column_name {
            set(column);
        }
    }
}

/// A small hard-coded schema used by the `debug` pseudo-URL to preview the
/// node widgets without a database.
pub fn sample() -> Schema {
    fn col(name: &str, ty: &str, pk: bool, fk: bool, self_fk: bool) -> Column {
        Column {
            name: name.into(),
            data_type: ty.into(),
            is_primary_key: pk,
            is_foreign_key: fk,
            is_self_foreign_key: self_fk,
        }
    }

    let tables = vec![
        Table {
            schema: "public".into(),
            name: "users".into(),
            columns: vec![
                col("id", "integer", true, false, false),
                col("name", "text", false, false, false),
                col("email", "text", false, false, false),
            ],
        },
        Table {
            schema: "public".into(),
            name: "posts".into(),
            columns: vec![
                col("id", "integer", true, false, false),
                col("user_id", "integer", false, true, false),
                col("title", "text", false, false, false),
            ],
        },
        Table {
            schema: "public".into(),
            name: "comments".into(),
            columns: vec![
                col("id", "integer", true, false, false),
                col("post_id", "integer", false, true, false),
                col("user_id", "integer", false, true, false),
            ],
        },
        Table {
            schema: "public".into(),
            name: "categories".into(),
            columns: vec![
                col("id", "integer", true, false, false),
                col("parent_id", "integer", false, true, true),
                col("name", "text", false, false, false),
            ],
        },
    ];

    let foreign_keys = vec![
        ForeignKey {
            from_schema: "public".into(),
            from_table: "posts".into(),
            to_schema: "public".into(),
            to_table: "users".into(),
            from_columns: vec!["user_id".into()],
        },
        ForeignKey {
            from_schema: "public".into(),
            from_table: "comments".into(),
            to_schema: "public".into(),
            to_table: "posts".into(),
            from_columns: vec!["post_id".into()],
        },
        ForeignKey {
            from_schema: "public".into(),
            from_table: "comments".into(),
            to_schema: "public".into(),
            to_table: "users".into(),
            from_columns: vec!["user_id".into()],
        },
        ForeignKey {
            from_schema: "public".into(),
            from_table: "categories".into(),
            to_schema: "public".into(),
            to_table: "categories".into(),
            from_columns: vec!["parent_id".into()],
        },
    ];

    Schema {
        tables,
        foreign_keys,
    }
}
