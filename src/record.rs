//! The `.rootstock.toml` record: the link between a project and its template.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::RelPath;

/// Name of the record file at the project root.
pub const RECORD_FILE: &str = ".rootstock.toml";

/// Schema version this build understands.
const SUPPORTED_SCHEMA: u32 = 1;

/// Answers by question name.
pub type Answers = BTreeMap<String, serde_json::Value>;

/// How rootstock treats an emitted file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileClass {
    Managed,
    Starter,
}

/// Every path rootstock emitted into the project, with its class.
pub type Inventory = BTreeMap<RelPath, FileClass>;

/// Parsed `.rootstock.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub schema: u32,
    pub source: String,
    /// Template subdirectory of the source; `None` when the template is the source itself.
    pub subdir: Option<String>,
    pub version: String,
    pub commit: String,
    pub answers: Answers,
    pub inventory: Inventory,
}

/// Errors from reading or writing the record.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("{}: not a rootstock project ({RECORD_FILE} not found)", dir.display())]
    NotAProject { dir: PathBuf },
    #[error("cannot read {}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: `{RECORD_FILE}` must be a regular file", path.display())]
    NotRegular { path: PathBuf },
    #[error("cannot write {}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid record {}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{}: unsupported record schema {found} (this rootstock supports schema {SUPPORTED_SCHEMA})", path.display())]
    UnsupportedSchema { path: PathBuf, found: u32 },
    #[error("answer `{question}` cannot be stored in the record: {reason}")]
    Answer { question: String, reason: String },
    #[error("cannot serialize record")]
    Serialize(#[from] toml::ser::Error),
}

/// On-disk shape: answers are TOML values (TOML has no null).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecord {
    schema: u32,
    source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subdir: Option<String>,
    version: String,
    commit: String,
    #[serde(default)]
    answers: BTreeMap<String, toml::Value>,
    #[serde(default)]
    inventory: BTreeMap<RelPath, FileClass>,
}

pub(crate) fn json_to_toml(
    question: &str,
    value: &serde_json::Value,
) -> Result<toml::Value, RecordError> {
    let fail = |reason: &str| RecordError::Answer {
        question: question.to_string(),
        reason: reason.to_string(),
    };
    Ok(match value {
        serde_json::Value::Null => return Err(fail("null cannot be stored (TOML has no null)")),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if n.is_u64() {
                return Err(fail("integer is out of range for TOML"));
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                return Err(fail("unsupported number"));
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(items) => toml::Value::Array(
            items
                .iter()
                .map(|v| json_to_toml(question, v))
                .collect::<Result<_, _>>()?,
        ),
        serde_json::Value::Object(map) => toml::Value::Table(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), json_to_toml(question, v)?)))
                .collect::<Result<_, RecordError>>()?,
        ),
    })
}

pub(crate) fn toml_to_json(
    question: &str,
    value: &toml::Value,
) -> Result<serde_json::Value, RecordError> {
    Ok(match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::from(*i),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| RecordError::Answer {
                question: question.to_string(),
                reason: "float is not a finite number".to_string(),
            })?,
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|v| toml_to_json(question, v))
                .collect::<Result<_, _>>()?,
        ),
        toml::Value::Table(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), toml_to_json(question, v)?)))
                .collect::<Result<_, RecordError>>()?,
        ),
    })
}

/// Reads the record of the project at `project`.
pub fn read(project: &Path) -> Result<Record, RecordError> {
    let path = project.join(RECORD_FILE);
    // The record is project content: a symlink could point anywhere.
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => return Err(RecordError::NotRegular { path }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(RecordError::NotAProject {
                dir: project.to_path_buf(),
            });
        }
        Err(source) => return Err(RecordError::Read { path, source }),
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) => return Err(RecordError::Read { path, source }),
    };
    parse(&path, &text)
}

/// Just the schema, read before the strict parse so that a future record is
/// reported by its schema, not by a field this build does not know.
#[derive(Deserialize)]
struct SchemaOnly {
    schema: u32,
}

fn parse(path: &Path, text: &str) -> Result<Record, RecordError> {
    let parse_error = |source| RecordError::Parse {
        path: path.to_path_buf(),
        source,
    };
    let head: SchemaOnly = toml::from_str(text).map_err(parse_error)?;
    if head.schema != SUPPORTED_SCHEMA {
        return Err(RecordError::UnsupportedSchema {
            path: path.to_path_buf(),
            found: head.schema,
        });
    }
    let raw: RawRecord = toml::from_str(text).map_err(parse_error)?;
    let answers = raw
        .answers
        .iter()
        .map(|(k, v)| Ok((k.clone(), toml_to_json(k, v)?)))
        .collect::<Result<_, RecordError>>()?;
    Ok(Record {
        schema: raw.schema,
        source: raw.source,
        subdir: raw.subdir,
        version: raw.version,
        commit: raw.commit,
        answers,
        inventory: raw.inventory,
    })
}

/// Serializes the record to the text of `.rootstock.toml`; fails on answers
/// TOML cannot store. Callers build it before writing anything else.
pub fn to_text(record: &Record) -> Result<String, RecordError> {
    let raw = RawRecord {
        schema: record.schema,
        source: record.source.clone(),
        subdir: record.subdir.clone(),
        version: record.version.clone(),
        commit: record.commit.clone(),
        answers: record
            .answers
            .iter()
            .map(|(k, v)| Ok((k.clone(), json_to_toml(k, v)?)))
            .collect::<Result<_, RecordError>>()?,
        inventory: record.inventory.clone(),
    };
    let mut text = toml::to_string(&raw)?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// Writes record text into `project`: to a temporary file in the same
/// directory, then renamed over `.rootstock.toml`.
pub fn write_text(project: &Path, text: &str) -> Result<(), RecordError> {
    let path = project.join(RECORD_FILE);
    let io = |source| RecordError::Write {
        path: path.clone(),
        source,
    };
    let mut tmp = tempfile::NamedTempFile::new_in(project).map_err(io)?;
    std::io::Write::write_all(&mut tmp, text.as_bytes()).map_err(io)?;
    set_mode(tmp.path()).map_err(io)?;
    tmp.persist(&path).map_err(|e| io(e.error))?;
    Ok(())
}

/// The record is an ordinary project file: 0644, not tempfile's 0600.
#[cfg(unix)]
fn set_mode(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Serializes and writes the record: `to_text` then `write_text`.
pub fn write(project: &Path, record: &Record) -> Result<(), RecordError> {
    write_text(project, &to_text(record)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Record {
        let mut answers = Answers::new();
        answers.insert("name".into(), json!("demo"));
        answers.insert("rust".into(), json!(true));
        answers.insert("count".into(), json!(3));
        answers.insert("ratio".into(), json!(1.5));
        answers.insert(
            "services".into(),
            json!({"web": {"port": 8080, "tags": ["a", "b"]}, "list": [1, 2]}),
        );
        let mut inventory = Inventory::new();
        inventory.insert("src/main.rs".into(), FileClass::Starter);
        inventory.insert("Cargo.toml".into(), FileClass::Managed);
        Record {
            schema: 1,
            source: "gh:owner/repo".into(),
            subdir: None,
            version: "v1.2.3".into(),
            commit: "abc123".into(),
            answers,
            inventory,
        }
    }

    #[test]
    fn subdir_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Record {
            subdir: Some("tpl".into()),
            ..sample()
        };
        write(dir.path(), &rec).unwrap();
        let text = std::fs::read_to_string(dir.path().join(RECORD_FILE)).unwrap();
        assert!(text.contains("subdir = \"tpl\""), "{text}");
        assert_eq!(read(dir.path()).unwrap(), rec);
    }

    #[test]
    fn record_without_subdir_reads_as_none_and_omits_it() {
        let dir = tempfile::tempdir().unwrap();
        let old = "schema = 1\nsource = \"gh:o/r\"\nversion = \"v1.0.0\"\ncommit = \"abc\"\n";
        std::fs::write(dir.path().join(RECORD_FILE), old).unwrap();
        let rec = read(dir.path()).unwrap();
        assert_eq!(rec.subdir, None);
        assert!(!to_text(&rec).unwrap().contains("subdir"));
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let rec = sample();
        write(dir.path(), &rec).unwrap();
        assert_eq!(read(dir.path()).unwrap(), rec);
    }

    #[test]
    fn writes_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        let rec = sample();
        write(dir.path(), &rec).unwrap();
        let first = std::fs::read(dir.path().join(RECORD_FILE)).unwrap();
        write(dir.path(), &rec).unwrap();
        let second = std::fs::read(dir.path().join(RECORD_FILE)).unwrap();
        assert_eq!(first, second);
        let text = String::from_utf8(first).unwrap();
        assert!(text.ends_with('\n'));
        let pos = |s: &str| text.find(s).unwrap();
        assert!(pos("count") < pos("name") && pos("name") < pos("ratio"));
        assert!(pos("Cargo.toml") < pos("src/main.rs"));
        assert!(pos("[answers]") < pos("[inventory]"));
    }

    #[test]
    fn rejects_null_answer() {
        let dir = tempfile::tempdir().unwrap();
        let mut rec = sample();
        rec.answers.insert("nested".into(), json!({"a": [null]}));
        let err = write(dir.path(), &rec).unwrap_err().to_string();
        assert!(err.contains("nested"), "{err}");
        let mut rec = sample();
        rec.answers.insert("top".into(), json!(null));
        let err = write(dir.path(), &rec).unwrap_err().to_string();
        assert!(err.contains("top"), "{err}");
        assert!(!dir.path().join(RECORD_FILE).exists());
    }

    #[test]
    fn missing_record_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = read(dir.path()).unwrap_err().to_string();
        assert!(err.contains(".rootstock.toml"), "{err}");
        assert!(err.contains("not a rootstock project"), "{err}");
    }

    #[test]
    fn rejects_unsupported_schema() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(RECORD_FILE),
            "schema = 2\nsource = \"s\"\nversion = \"v1.0.0\"\ncommit = \"c\"\n",
        )
        .unwrap();
        let err = read(dir.path()).unwrap_err().to_string();
        assert!(err.contains("unsupported record schema 2"), "{err}");
        assert!(err.contains(".rootstock.toml"), "{err}");
    }

    #[test]
    fn to_text_then_write_text_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let rec = sample();
        let text = to_text(&rec).unwrap();
        write_text(dir.path(), &text).unwrap();
        assert_eq!(read(dir.path()).unwrap(), rec);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(RECORD_FILE)).unwrap(),
            text
        );
    }

    #[test]
    fn to_text_rejects_null_without_writing() {
        let mut rec = sample();
        rec.answers.insert("nested".into(), json!({"a": null}));
        let err = to_text(&rec).unwrap_err().to_string();
        assert!(err.contains("nested"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn record_file_mode_is_0644() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &sample()).unwrap();
        let mode = std::fs::metadata(dir.path().join(RECORD_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
    }

    #[test]
    fn record_future_schema_message() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(RECORD_FILE),
            "schema = 2\nsource = \"s\"\nversion = \"v1.0.0\"\ncommit = \"c\"\nfuture = 1\n",
        )
        .unwrap();
        let err = read(dir.path()).unwrap_err().to_string();
        assert!(err.contains("unsupported record schema 2"), "{err}");
        assert!(!err.contains("future"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn record_symlink_rejected() {
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), &sample()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            outside.path().join(RECORD_FILE),
            dir.path().join(RECORD_FILE),
        )
        .unwrap();
        let err = read(dir.path()).unwrap_err().to_string();
        assert!(
            err.contains("`.rootstock.toml` must be a regular file"),
            "{err}"
        );
    }

    #[test]
    fn parse_error_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(RECORD_FILE), "schema = [").unwrap();
        let err = read(dir.path()).unwrap_err().to_string();
        assert!(err.contains(".rootstock.toml"), "{err}");
    }
}
