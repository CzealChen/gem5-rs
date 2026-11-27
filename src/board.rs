//! Assemble immutable board configurations and run fresh simulator instances.

mod graph;
mod run;

use graph::{Graph, Plan};
use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)))]
pub enum ConfigError {
    #[snafu(display("{message}"))]
    Topology { message: String },
    #[snafu(display("{location}: {source}"))]
    Context {
        location: String,
        source: Box<ConfigError>,
    },
    #[snafu(display("{location}: {source}"))]
    Decode {
        location: String,
        source: serde_json::Error,
    },
    #[snafu(display("{location}: {source}"))]
    Image {
        location: String,
        source: crate::elf::ElfError,
    },
}
impl From<String> for ConfigError {
    fn from(message: String) -> Self {
        Self::Topology { message }
    }
}
impl From<&str> for ConfigError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

#[derive(Debug, Snafu)]
pub enum RunError {
    #[snafu(display("{message}"))]
    Simulation { message: String },
    #[snafu(display("{location}: {source}"))]
    Engine {
        location: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[snafu(display("encoding simulation result: {source}"))]
    Encode { source: serde_json::Error },
}
impl From<String> for RunError {
    fn from(message: String) -> Self {
        Self::Simulation { message }
    }
}

pub struct Configuration(Plan);

impl Configuration {
    pub fn build(json: &str) -> Result<Self, ConfigError> {
        Ok(Self(Graph::parse(json)?.plan()?))
    }
    pub fn run(&self, events: u64) -> Result<String, RunError> {
        run::run(&self.0, events)
    }
}

pub fn validate(graph: &str) -> Result<(), ConfigError> {
    Configuration::build(graph).map(drop)
}

pub fn run(graph: &str, events: u64) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Configuration::build(graph)?.run(events)?)
}

#[cfg(test)]
mod tests;
