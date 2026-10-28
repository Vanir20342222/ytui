//! A bounded, read-only SQLite 3 file reader: enough of the file format to
//! scan whole tables (as `SELECT columns FROM table` does) for browser cookie
//! databases. The write-ahead log is not read, as upstream's copied database
//! is opened without it.
use crate::{Error, Result};

const HEADER: &[u8; 16] = b"SQLite format 3\0";
/// Databases larger than this are refused.
const MAX_FILE: usize = 256 * 1024 * 1024;

/// One column value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Real(value) => Some(*value as i64),
            _ => None,
        }
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(value) => Some(*value as f64),
            Self::Real(value) => Some(*value),
            _ => None,
        }
    }

    /// SQLite truthiness of an integer column.
    pub(crate) fn truthy(&self) -> bool {
        self.as_f64().is_some_and(|value| value != 0.0)
    }
}

fn corrupt(message: &str) -> Error {
    Error::Cookies(format!("malformed SQLite database: {message}"))
}

pub(crate) struct Database {
    bytes: Vec<u8>,
    page_size: usize,
    usable: usize,
    utf16: Option<bool>,
}

/// A table's column names (from its `CREATE TABLE` statement) and rows.
pub(crate) struct Table {
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<Value>>,
}

impl Table {
    pub(crate) fn column(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| column.eq_ignore_ascii_case(name))
    }
}

impl Database {
    pub(crate) fn open(path: &std::path::Path) -> Result<Self> {
        let metadata = std::fs::metadata(path)?;
        if metadata.len() > MAX_FILE as u64 {
            return Err(Error::Cookies("cookie database exceeds 256 MiB".into()));
        }
        Self::from_bytes(std::fs::read(path)?)
    }

    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < 100 || &bytes[..16] != HEADER {
            return Err(Error::Cookies("file is not a database".into()));
        }
        let page_size = match u16::from_be_bytes([bytes[16], bytes[17]]) {
            1 => 65536,
            size if size >= 512 && size.is_power_of_two() => usize::from(size),
            _ => return Err(corrupt("invalid page size")),
        };
        let reserved = usize::from(bytes[20]);
        let usable = page_size
            .checked_sub(reserved)
            .filter(|usable| *usable >= 480)
            .ok_or_else(|| corrupt("invalid reserved space"))?;
        let utf16 = match u32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]) {
            0 | 1 => None,
            2 => Some(false),
            3 => Some(true),
            _ => return Err(corrupt("invalid text encoding")),
        };
        Ok(Self {
            bytes,
            page_size,
            usable,
            utf16,
        })
    }

    /// `PRAGMA user_version`.
    pub(crate) fn user_version(&self) -> i64 {
        i64::from(i32::from_be_bytes([
            self.bytes[60],
            self.bytes[61],
            self.bytes[62],
            self.bytes[63],
        ]))
    }

    fn page(&self, number: u32) -> Result<&[u8]> {
        let index = usize::try_from(number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .ok_or_else(|| corrupt("page number 0"))?;
        let start = index
            .checked_mul(self.page_size)
            .ok_or_else(|| corrupt("page number overflow"))?;
        self.bytes
            .get(start..start + self.page_size)
            .ok_or_else(|| corrupt("page beyond the end of the file"))
    }

    /// Every row of the table b-tree rooted at `root`, in rowid order:
    /// (rowid, record payload).
    fn scan(&self, root: u32) -> Result<Vec<(i64, Vec<u8>)>> {
        let mut rows = Vec::new();
        let mut stack = vec![root];
        let mut visited = 0usize;
        let limit = self.bytes.len() / self.page_size + 1;
        while let Some(number) = stack.pop() {
            visited += 1;
            if visited > limit {
                return Err(corrupt("b-tree cycle"));
            }
            let page = self.page(number)?;
            let offset = if number == 1 { 100 } else { 0 };
            let kind = page[offset];
            let cells = usize::from(u16::from_be_bytes([page[offset + 3], page[offset + 4]]));
            let header = if kind == 0x05 { 12 } else { 8 };
            let pointers = offset + header;
            let cell = |index: usize| -> Result<usize> {
                let at = pointers + 2 * index;
                let bytes = page
                    .get(at..at + 2)
                    .ok_or_else(|| corrupt("cell pointer"))?;
                Ok(usize::from(u16::from_be_bytes([bytes[0], bytes[1]])))
            };
            match kind {
                0x05 => {
                    // Interior: children left to right, then the right-most
                    // pointer (pushed in reverse for in-order traversal).
                    let right =
                        u32::from_be_bytes(page[offset + 8..offset + 12].try_into().unwrap());
                    stack.push(right);
                    for index in (0..cells).rev() {
                        let at = cell(index)?;
                        let child = page
                            .get(at..at + 4)
                            .ok_or_else(|| corrupt("interior cell"))?;
                        stack.push(u32::from_be_bytes(child.try_into().unwrap()));
                    }
                }
                0x0d => {
                    for index in 0..cells {
                        let at = cell(index)?;
                        let (length, used) = varint(page.get(at..).unwrap_or_default())?;
                        let (rowid, used2) = varint(page.get(at + used..).unwrap_or_default())?;
                        let start = at + used + used2;
                        let payload = self.payload(page, start, length as usize)?;
                        rows.push((rowid as i64, payload));
                    }
                }
                _ => return Err(corrupt("not a table b-tree page")),
            }
        }
        Ok(rows)
    }

    /// A leaf cell's payload with its overflow chain.
    fn payload(&self, page: &[u8], start: usize, length: usize) -> Result<Vec<u8>> {
        if length > MAX_FILE {
            return Err(corrupt("oversized record"));
        }
        let usable = self.usable;
        let max_local = usable - 35;
        let local = if length <= max_local {
            length
        } else {
            let min_local = (usable - 12) * 32 / 255 - 23;
            let k = min_local + (length - min_local) % (usable - 4);
            if k <= max_local { k } else { min_local }
        };
        let mut payload = page
            .get(start..start + local)
            .ok_or_else(|| corrupt("cell payload"))?
            .to_vec();
        if local < length {
            let at = start + local;
            let mut next = u32::from_be_bytes(
                page.get(at..at + 4)
                    .ok_or_else(|| corrupt("overflow pointer"))?
                    .try_into()
                    .unwrap(),
            );
            while payload.len() < length {
                let overflow = self.page(next)?;
                next = u32::from_be_bytes(overflow[..4].try_into().unwrap());
                let take = (length - payload.len()).min(usable - 4);
                payload.extend_from_slice(&overflow[4..4 + take]);
                if payload.len() < length && next == 0 {
                    return Err(corrupt("truncated overflow chain"));
                }
            }
        }
        Ok(payload)
    }

    fn record(&self, payload: &[u8]) -> Result<Vec<Value>> {
        let (header_length, mut at) = varint(payload)?;
        let header_length = header_length as usize;
        let mut body = header_length;
        let mut values = Vec::new();
        while at < header_length {
            let (serial, used) = varint(payload.get(at..header_length).unwrap_or_default())?;
            at += used;
            let size = match serial {
                0 | 8 | 9 | 10 | 11 => 0,
                1..=4 => serial as usize,
                5 => 6,
                6 | 7 => 8,
                serial => ((serial - 12) / 2) as usize,
            };
            let bytes = payload
                .get(body..body + size)
                .ok_or_else(|| corrupt("record value"))?;
            body += size;
            let signed = |bytes: &[u8]| {
                let mut value = if bytes[0] & 0x80 != 0 { -1i64 } else { 0 };
                for byte in bytes {
                    value = (value << 8) | i64::from(*byte);
                }
                value
            };
            values.push(match serial {
                0 | 10 | 11 => Value::Null,
                1..=6 => Value::Integer(signed(bytes)),
                7 => Value::Real(f64::from_bits(u64::from_be_bytes(
                    bytes.try_into().unwrap(),
                ))),
                8 => Value::Integer(0),
                9 => Value::Integer(1),
                serial if serial % 2 == 0 => Value::Blob(bytes.to_vec()),
                _ => Value::Text(self.text(bytes)),
            });
        }
        Ok(values)
    }

    fn text(&self, bytes: &[u8]) -> String {
        match self.utf16 {
            None => String::from_utf8_lossy(bytes).into_owned(),
            Some(big_endian) => {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|unit| {
                        if big_endian {
                            u16::from_be_bytes(*unit)
                        } else {
                            u16::from_le_bytes(*unit)
                        }
                    })
                    .collect();
                String::from_utf16_lossy(&units)
            }
        }
    }

    /// `SELECT * FROM name`: the table's columns and rows. Rows written
    /// before an `ALTER TABLE ADD COLUMN` lack trailing values (NULL here);
    /// an `INTEGER PRIMARY KEY` column reads as the rowid.
    pub(crate) fn table(&self, name: &str) -> Result<Option<Table>> {
        let mut found = None;
        for (_, payload) in self.scan(1)? {
            let row = self.record(&payload)?;
            let text = |index: usize| row.get(index).and_then(Value::as_text);
            if text(0) == Some("table")
                && text(1).is_some_and(|table| table.eq_ignore_ascii_case(name))
            {
                let root = row
                    .get(3)
                    .and_then(Value::as_i64)
                    .and_then(|root| u32::try_from(root).ok())
                    .ok_or_else(|| corrupt("table root page"))?;
                found = Some((root, text(4).unwrap_or_default().to_owned()));
                break;
            }
        }
        let Some((root, sql)) = found else {
            return Ok(None);
        };
        let (columns, rowid_alias) = columns(&sql);
        let mut rows = Vec::new();
        for (rowid, payload) in self.scan(root)? {
            let mut values = self.record(&payload)?;
            values.resize(columns.len().max(values.len()), Value::Null);
            if let Some(alias) = rowid_alias
                && values.get(alias) == Some(&Value::Null)
            {
                values[alias] = Value::Integer(rowid);
            }
            rows.push(values);
        }
        Ok(Some(Table { columns, rows }))
    }
}

/// A SQLite varint and its length.
fn varint(bytes: &[u8]) -> Result<(u64, usize)> {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().take(9).enumerate() {
        if index == 8 {
            return Ok(((value << 8) | u64::from(*byte), 9));
        }
        value = (value << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(corrupt("truncated varint"))
}

/// Column names from `CREATE TABLE name (definitions)`, and the index of an
/// `INTEGER PRIMARY KEY` column.
fn columns(sql: &str) -> (Vec<String>, Option<usize>) {
    let (Some(open), Some(close)) = (sql.find('('), sql.rfind(')')) else {
        return (Vec::new(), None);
    };
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    let mut quote = None;
    for character in sql[open + 1..close].chars() {
        match (quote, character) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`') => quote = Some(character),
            (None, '[') => quote = Some(']'),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    parts.push(current);
    let mut names = Vec::new();
    let mut alias = None;
    for part in parts {
        let part = part.trim();
        let first = part
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(
            first.as_str(),
            "CONSTRAINT" | "PRIMARY" | "UNIQUE" | "CHECK" | "FOREIGN"
        ) {
            continue;
        }
        let name = match part.chars().next() {
            Some(open @ ('"' | '`' | '[')) => {
                let close = if open == '[' { ']' } else { open };
                part[1..].split(close).next().unwrap_or_default().to_owned()
            }
            _ => part
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned(),
        };
        let rest =
            part[part.find(char::is_whitespace).unwrap_or(part.len())..].to_ascii_uppercase();
        let words: Vec<&str> = rest.split_whitespace().collect();
        if words.first() == Some(&"INTEGER")
            && words.windows(2).any(|pair| pair == ["PRIMARY", "KEY"])
        {
            alias = Some(names.len());
        }
        names.push(name);
    }
    (names, alias)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_and_column_definitions() {
        assert_eq!(varint(&[0x7f]).unwrap(), (127, 1));
        assert_eq!(varint(&[0x81, 0x00]).unwrap(), (128, 2));
        let (names, alias) = columns(
            "CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, \"name\" TEXT, value TEXT DEFAULT 'a,b', \
             CONSTRAINT moz_uniqueid UNIQUE (name, host))",
        );
        assert_eq!(names, ["id", "name", "value"]);
        assert_eq!(alias, Some(0));
    }
}
