use crate::board;
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
};
use std::error::Error;

pyo3::create_exception!(gem5rs._native, ConfigurationError, PyValueError);
pyo3::create_exception!(gem5rs._native, SimulationError, PyRuntimeError);

fn chain(py: Python<'_>, error: &dyn Error, exception: fn(String) -> PyErr) -> PyErr {
    let outer = exception(error.to_string());
    if let Some(source) = error.source() {
        outer.set_cause(py, Some(chain(py, source, exception)));
    }
    outer
}

#[pyclass(frozen, module = "gem5rs._native")]
struct Configuration(board::Configuration);

#[pymethods]
impl Configuration {
    fn run(&self, py: Python<'_>, events: u64) -> PyResult<String> {
        self.0
            .run(events)
            .map_err(|e| chain(py, &e, SimulationError::new_err))
    }
}

#[pyfunction]
fn build(py: Python<'_>, graph: &str) -> PyResult<Configuration> {
    board::Configuration::build(graph)
        .map(Configuration)
        .map_err(|e| chain(py, &e, ConfigurationError::new_err))
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add(
        "ConfigurationError",
        module.py().get_type::<ConfigurationError>(),
    )?;
    module.add("SimulationError", module.py().get_type::<SimulationError>())?;
    module.add_class::<Configuration>()?;
    module.add_function(wrap_pyfunction!(build, module)?)?;
    Ok(())
}
