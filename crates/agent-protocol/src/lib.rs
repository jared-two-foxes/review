use std::collections::VecDeque;

/// An injectable source of application time.
pub trait Clock {
    fn now(&self) -> &str;
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
    fn now(&self) -> &str {
        &self.value
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
