use std::collections::HashSet;
use std::fs::File;
use std::io::{Error as IoError, Read, Write};
use std::path::Path;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitCode {
    Approved = 0,
    ChangesRequested = 1,
    InvalidRequest = 2,
    Indeterminate = 3,
    InternalFailure = 4,
    Cancellation = 5,
}

pub fn read_request(path: &Path) -> Result<Vec<u8>, IoError> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    Ok(buffer)
}

pub fn write_json_stdout<T: serde::Serialize>(value: &T) -> Result<(), IoError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer(&mut handle, value)?;
    writeln!(handle)?; // newline terminator
    Ok(())
}

pub fn write_stderr(msg: &str) -> Result<(), IoError> {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    writeln!(handle, "{}", msg)?;
    Ok(())
}

#[derive(Debug)]
pub enum ParseError {
    Syntax(serde_json::Error),
    DuplicateKey(String),
}

pub fn parse_strict_json<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T, ParseError> {
    let mut de = serde_json::Deserializer::from_slice(data);
    let value = T::deserialize(&mut de).map_err(ParseError::Syntax)?;
    de.end().map_err(ParseError::Syntax)?;
    Ok(value)
}
