use std::collections::VecDeque;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

/// An injectable source of application time.
pub trait Clock {
    fn now(&self) -> String;
}

/// An injectable source of identifiers.
pub trait IdGenerator {
    fn next_id(&mut self) -> String;
}

/// Fixed clock used by deterministic protocol tests.
#[derive(Clone, Debug)]
pub struct FixedClock {
    value: String,
}

impl FixedClock {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> String {
        self.value.clone()
    }
}

/// Production wall-clock source returning ISO-8601 UTC timestamps.
#[derive(Clone, Debug, Default)]
pub struct SystemClock;

impl SystemClock {
    pub fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now(&self) -> String {
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("system UTC time is RFC3339-formatable")
    }
}

/// Predictable sequential identifier source used by deterministic tests.
#[derive(Clone, Debug)]
pub struct SequenceIdGenerator {
    ids: VecDeque<String>,
}

impl SequenceIdGenerator {
    pub fn new(ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            ids: ids.into_iter().map(Into::into).collect(),
        }
    }
}

impl IdGenerator for SequenceIdGenerator {
    fn next_id(&mut self) -> String {
        self.ids.pop_front().unwrap_or_default()
    }
}

/// Production identifier source producing UUID v4 strings.
#[derive(Clone, Debug, Default)]
pub struct RandomIdGenerator;

impl RandomIdGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl IdGenerator for RandomIdGenerator {
    fn next_id(&mut self) -> String {
        Uuid::new_v4().to_string()
    }
}
