use crate::dsl::Step;

/// Trait for executing workflow steps.
///
/// This core crate does not prescribe how steps are run, it merely
/// defines the operations available. Implementors are expected to
/// provide the actual browser automation logic in another crate.
pub trait Executor {
    type Error;

    /// Execute a single workflow step.
    fn execute_step(&mut self, step: &Step) -> Result<(), Self::Error>;
}
