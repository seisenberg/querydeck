/// Split a SQL script into individual statements.
///
/// Handles: line comments (`--`, `#` is NOT treated as a comment since it is
/// only meaningful in MySQL and conflicts with other dialects), block comments
/// (`/* */`, nested), single-quoted strings with `''` escapes and backslash
/// escapes, double-quoted identifiers, MySQL backticks, square-bracket
/// identifiers, and PostgreSQL dollar-quoting (`$tag$ ... $tag$`).
pub fn split_statements(script: &str) -> Vec<Statement> {
    let bytes = script.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let n = bytes.len();

    while i < n {
        let c = bytes[i];
        match c {
            b'-' if i + 1 < n && bytes[i + 1] == b'-' => {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                let mut depth = 1;
                i += 2;
                while i < n && depth > 0 {
                    if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'\'' => {
                i += 1;
                while i < n {
                    if bytes[i] == b'\\' && i + 1 < n {
                        i += 2;
                    } else if bytes[i] == b'\'' {
                        if i + 1 < n && bytes[i + 1] == b'\'' {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'"' | b'`' => {
                let q = c;
                i += 1;
                while i < n {
                    if bytes[i] == q {
                        if i + 1 < n && bytes[i + 1] == q {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'[' => {
                i += 1;
                while i < n && bytes[i] != b']' {
                    i += 1;
                }
                i += 1;
            }
            b'$' => {
                // possible dollar-quote: $tag$
                let tag_start = i + 1;
                let mut j = tag_start;
                while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                if j < n && bytes[j] == b'$' {
                    let tag = &script[i..=j]; // "$tag$"
                    if let Some(end) = script[j + 1..].find(tag) {
                        i = j + 1 + end + tag.len();
                    } else {
                        i = n;
                    }
                } else {
                    i += 1;
                }
            }
            b';' => {
                let text = script[start..i].trim();
                if !text.is_empty() {
                    out.push(Statement { sql: text.to_string(), offset: start });
                }
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    let text = script[start..].trim();
    if !text.is_empty() {
        out.push(Statement { sql: text.to_string(), offset: start });
    }
    out
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub sql: String,
    pub offset: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_basic() {
        let s = split_statements("select 1; select 2;");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].sql, "select 1");
    }

    #[test]
    fn respects_strings_and_comments() {
        let s = split_statements("select 'a;b'; -- c;\nselect \"x;y\" /* ; */; select `p;q`");
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn dollar_quoting() {
        let s = split_statements("create function f() returns int as $body$ begin return 1; end $body$ language plpgsql; select 1");
        assert_eq!(s.len(), 2);
    }
}
