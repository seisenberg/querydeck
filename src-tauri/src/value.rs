use serde::Serialize;

/// A single cell value, normalized for display/transfer.
/// Serializes untagged: null / bool / number / string.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum DbValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl DbValue {
    pub fn float(f: f64) -> Self {
        if f.is_finite() {
            DbValue::Float(f)
        } else {
            DbValue::Text(f.to_string())
        }
    }

    pub fn bytes(b: &[u8]) -> Self {
        const MAX: usize = 64;
        let shown = &b[..b.len().min(MAX)];
        let hex: String = shown.iter().map(|x| format!("{x:02x}")).collect();
        if b.len() > MAX {
            DbValue::Text(format!("0x{hex}… ({} bytes)", b.len()))
        } else {
            DbValue::Text(format!("0x{hex}"))
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, DbValue::Null)
    }

    /// Plain-text rendering (exports, SQL literal building).
    pub fn as_text(&self) -> String {
        match self {
            DbValue::Null => String::new(),
            DbValue::Bool(b) => b.to_string(),
            DbValue::Int(i) => i.to_string(),
            DbValue::Float(f) => f.to_string(),
            DbValue::Text(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnMeta {
    pub name: String,
    #[serde(rename = "typeName")]
    pub type_name: String,
}

impl ColumnMeta {
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self { name: name.into(), type_name: type_name.into() }
    }
}
