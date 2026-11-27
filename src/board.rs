//! Assemble native boards and translate simulator results into the JSON protocol.

mod graph;
mod run;

use graph::Graph;

pub fn validate(graph: &str) -> Result<(), String> {
    Graph::parse(graph)?.plan().map(|_| ())
}

pub fn run(graph: &str, events: u64) -> Result<String, String> {
    let graph = Graph::parse(graph)?;
    run::run(graph.plan()?, events)
}

#[cfg(test)]
mod tests;
