use serde::{Deserialize, Serialize};

/// Introspected database structure, shaped for the schema navigator tree.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub schemas: Vec<SchemaInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub name: String,
    pub tables: Vec<TableInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    /// "table" | "view" | "materialized view" | "foreign table" | "system table"
    pub kind: String,
    pub columns: Vec<ColumnInfo>,
    /// Primary-key column names, in order.
    pub pk: Vec<String>,
    /// Index definitions (SQL or descriptive), for DDL generation.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub indexes: Vec<String>,
    /// Foreign-key constraint definitions, for DDL generation.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub foreign_keys: Vec<String>,
}

impl TableInfo {
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            columns: Vec::new(),
            pk: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    #[serde(rename = "dataType")]
    pub data_type: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}
