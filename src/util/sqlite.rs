use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose};
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Map, Value as JsonValue, json};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

/// Quotes a SQLite identifier safely by doubling embedded double quotes.
fn quote_identifier(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Reads an in-memory SQLite database and exports its tables to a JSON file.
///
/// Blob data is encoded as a Base64 string within a special JSON object: `{"__base64": "..."}`.
pub fn export_sqlite_to_json(db_bytes: &[u8], out_json_path: &Path) -> Result<()> {
    let mut temp_db_file =
        NamedTempFile::new().context("Failed to create temporary file for SQLite DB")?;
    temp_db_file
        .write_all(db_bytes)
        .context("Failed to write SQLite bytes to temporary file")?;
    temp_db_file
        .flush()
        .context("Failed to flush SQLite bytes to temporary file")?;
    let temp_db_path = temp_db_file.into_temp_path();

    let conn = Connection::open(&temp_db_path)
        .context("Failed to open SQLite database from temporary file")?;

    let mut tables_stmt = conn.prepare(
        "SELECT name
         FROM sqlite_master
         WHERE type='table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name;",
    )?;
    let table_names: Vec<String> = tables_stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut export_data = Map::new();

    for table_name in table_names {
        let sql = format!("SELECT * FROM {}", quote_identifier(&table_name));
        let mut table_stmt = conn.prepare(&sql)?;
        let column_names: Vec<String> = table_stmt
            .column_names()
            .into_iter()
            .map(String::from)
            .collect();

        let rows_iter = table_stmt.query_map([], |row| {
            let mut row_map = Map::new();
            for (i, col_name) in column_names.iter().enumerate() {
                let value = row.get_ref(i)?;
                let json_value = match value {
                    ValueRef::Null => JsonValue::Null,
                    ValueRef::Integer(i) => json!(i),
                    ValueRef::Real(f) => json!(f),
                    ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    ValueRef::Blob(b) => {
                        json!({
                            "__base64": general_purpose::STANDARD.encode(b)
                        })
                    }
                };
                row_map.insert(col_name.clone(), json_value);
            }
            Ok(JsonValue::Object(row_map))
        })?;

        let rows: Vec<JsonValue> = rows_iter.collect::<Result<_, _>>()?;
        export_data.insert(table_name, JsonValue::Array(rows));
    }

    let final_json = json!({ "tables": export_data });

    if let Some(parent) = out_json_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).context("Failed to create parent directory for JSON output")?;
    }
    let out_file = File::create(out_json_path).with_context(|| {
        format!(
            "Failed to create JSON output file at {}",
            out_json_path.display()
        )
    })?;

    serde_json::to_writer_pretty(out_file, &final_json)
        .context("Failed to write JSON to output file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;
    use rusqlite::Connection;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_export_blob_to_json() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        {
            let conn = Connection::open(&db_path)?;
            conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, data BLOB)", [])?;
            let blob_data = vec![0xDE, 0xAD, 0xBE, 0xEF];
            conn.execute("INSERT INTO test (data) VALUES (?1)", [&blob_data])?;
        }

        let db_bytes = fs::read(&db_path)?;
        let json_path = temp_dir.path().join("out.json");
        export_sqlite_to_json(&db_bytes, &json_path)?;

        let json_content = fs::read_to_string(&json_path)?;
        let json: serde_json::Value = serde_json::from_str(&json_content)?;

        let table = &json["tables"]["test"];
        assert!(table.is_array());
        let row = &table[0];
        let blob_json = &row["data"];

        assert!(blob_json.is_object());
        let base64_str = blob_json["__base64"].as_str().unwrap();

        let decoded = general_purpose::STANDARD.decode(base64_str)?;
        assert_eq!(decoded, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        Ok(())
    }

    #[test]
    fn test_export_table_with_quoted_name() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("test.db");

        {
            let conn = Connection::open(&db_path)?;
            conn.execute(r#"CREATE TABLE "odd""table" (value TEXT)"#, [])?;
            conn.execute(r#"INSERT INTO "odd""table" (value) VALUES ('hello')"#, [])?;
        }

        let db_bytes = fs::read(&db_path)?;
        let json_path = temp_dir.path().join("out.json");
        export_sqlite_to_json(&db_bytes, &json_path)?;

        let json_content = fs::read_to_string(&json_path)?;
        let json: serde_json::Value = serde_json::from_str(&json_content)?;

        assert_eq!(json["tables"]["odd\"table"][0]["value"], "hello");

        Ok(())
    }
}
