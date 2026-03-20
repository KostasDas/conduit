//! # Conduit
//!
//! `conduit` is a type-safe, zero-cost pipeline engine designed for structured
//! data transformation.
//!
//! It uses a recursive, static-dispatch architecture to ensure that the
//! entire transformation chain is validated at compile-time with no
//! runtime overhead.

/// Represents the various error states a pipeline stage can encounter.
#[derive(Debug)]
pub enum PipelineError {
    /// An error that might be resolved by retrying the operation (e.g., a network timeout).
    Recoverable(String),
    /// An error that cannot be resolved by retrying (e.g., a validation failure).
    Permanent(String),
}

/// The core trait for all transformation logic.
///
/// Any struct implementing `Step` can be plugged into a [`Pipeline`].
pub trait Step {
    /// The type of data this stage accepts.
    type Input;
    /// The type of data this stage produces.
    type Output;

    /// Executes the logic for this specific stage.
    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError>;
    /// Decorates this step with a policy
    fn with<P>(self, policy: P) -> P::Decorated
    where
        P: Policy<Self>,
        Self: Sized,
    {
        policy.apply(self)
    }
}
/// Policy trait, decorates a step with some extra behavior, such as retrying or logging
///
/// Any struct implementing `Policy` can be plugged into a [`Pipeline`].
pub trait Policy<S: Step> {
    type Decorated: Step<Input = S::Input, Output = S::Output>;
    fn apply(self, step: S) -> Self::Decorated;
}
/// A completed execution chain that can process data.
///
/// Use [`Pipeline::builder`] to construct a new instance.
pub struct Pipeline<S> {
    steps: S,
}

impl Pipeline<()> {
    /// Starts the construction of a new pipeline.
    ///
    /// To ensure type safety, the user must specify the initial input type:
    /// `Pipeline::builder::<i32>()`
    pub fn builder<I>() -> PipelineBuilder<NoOp<I>> {
        PipelineBuilder {
            start_node: NoOp::new(),
        }
    }
}

impl<S> Pipeline<S>
where
    S: Step,
{
    /// Feeds data into the start of the pipeline and returns the final result.
    ///
    /// This method executes all stages sequentially. Execution stops upon completion
    /// or on PipelineError
    pub fn run(&self, input: S::Input) -> Result<S::Output, PipelineError> {
        self.steps.execute(input)
    }
}

impl<S> Pipeline<S>
where
    S: Step + 'static,
{
    /// Erases the concrete type of the pipeline, returning a boxed trait object.
    /// This is useful for storing different pipelines in a single collection as long as they share the same
    /// Input and Output types. The matching types can be bypassed by creating an
    /// Enum Wrapper of known Stages.
    pub fn into_boxed(self) -> Box<dyn Step<Input = S::Input, Output = S::Output>> {
        Box::new(self.steps)
    }
}

/// A utility for assembling [`Step`] implementations into a linear chain.
pub struct PipelineBuilder<S> {
    start_node: S,
}

impl<S> PipelineBuilder<S>
where
    S: Step,
{
    /// Appends a new stage to the end of the current pipeline.
    pub fn add_stage<A>(self, action: A) -> PipelineBuilder<PipelineStep<S, A>>
    where
        A: Step<Input = S::Output>,
    {
        let step = PipelineStep {
            current_step: self.start_node,
            next_step: action,
        };
        PipelineBuilder { start_node: step }
    }

    /// Appends a closure-based transformation to the pipeline.
    pub fn add_map<F, O>(
        self,
        f: F,
    ) -> PipelineBuilder<PipelineStep<S, ClosureStep<F, S::Output, O>>>
    where
        F: Fn(S::Output) -> Result<O, PipelineError>,
    {
        let wrapper = ClosureStep::new(f);
        self.add_stage(wrapper)
    }

    /// Seals the pipeline and returns a runnable [`Pipeline`] instance.
    ///
    /// This finalizes the internal recursive structure and adds a termination node.
    pub fn build(self) -> Pipeline<impl Step<Input = S::Input, Output = S::Output>> {
        let final_chain = PipelineStep {
            current_step: self.start_node,
            next_step: NoOp::new(),
        };

        Pipeline { steps: final_chain }
    }
}

/// An internal wrapper that links two stages together.
#[doc(hidden)]
pub struct PipelineStep<Current, Next> {
    current_step: Current,
    next_step: Next,
}

impl<Current, Next> Step for PipelineStep<Current, Next>
where
    Current: Step,
    Next: Step<Input = Current::Output>,
{
    type Input = Current::Input;
    type Output = Next::Output;
    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
        let res = self.current_step.execute(input)?;
        self.next_step.execute(res)
    }
}
#[doc(hidden)]
pub struct ClosureStep<F, I, O> {
    closure: F,
    _market: std::marker::PhantomData<(I, O)>,
}

impl<F, I, O> ClosureStep<F, I, O>
where
    F: Fn(I) -> Result<O, PipelineError>,
{
    fn new(closure: F) -> Self {
        ClosureStep {
            closure,
            _market: std::marker::PhantomData,
        }
    }
}

impl<F, I, O> Step for ClosureStep<F, I, O>
where
    F: Fn(I) -> Result<O, PipelineError>,
{
    type Input = I;
    type Output = O;
    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
        (self.closure)(input)
    }
}

/// An internal marker stage that terminates or starts a pipeline chain.
#[doc(hidden)]
pub struct NoOp<T> {
    _marker: std::marker::PhantomData<T>,
}

impl<T> NoOp<T> {
    fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> Step for NoOp<T> {
    type Input = T;
    type Output = T;

    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
        Ok(input)
    }
}

/// A configuration for retrying a [`Step`] when encountering recoverable errors.
pub struct Retry {
    max_retries: usize,
}

impl Retry {
    /// Configures a policy to retry a step a specific number of times.
    pub fn times(n: usize) -> Self {
        Self { max_retries: n }
    }
}

impl<S: Step> Policy<S> for Retry
where
    S::Input: Clone,
{
    type Decorated = RetryStep<S>;
    fn apply(self, step: S) -> Self::Decorated {
        RetryStep {
            max_retries: 1 + self.max_retries,
            inner: step,
        }
    }
}

/// An internal wrapper that executes retry logic for a decorated step.
#[doc(hidden)]
pub struct RetryStep<S> {
    inner: S,
    max_retries: usize,
}

impl<S> Step for RetryStep<S>
where
    S: Step,
    S::Input: Clone,
{
    type Input = S::Input;
    type Output = S::Output;

    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
        let mut last_err = None;
        for _ in 0..self.max_retries {
            match self.inner.execute(input.clone()) {
                Ok(output) => return Ok(output),
                Err(PipelineError::Permanent(e)) => return Err(PipelineError::Permanent(e)),
                Err(PipelineError::Recoverable(e)) => {
                    last_err = Some(PipelineError::Recoverable(e))
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            PipelineError::Permanent("Retry logic exhausted with no attempts".to_string())
        }))
    }
}
pub mod prelude {
    pub use crate::{Pipeline, PipelineBuilder, PipelineError, Policy, Retry, Step};
}
#[cfg(test)]
mod tests {
    use super::*;

    struct MultiplyByTwo;
    struct SubtractTen;

    impl Step for MultiplyByTwo {
        type Input = i32;
        type Output = i32;
        fn execute(&self, input: i32) -> Result<i32, PipelineError> {
            Ok(input * 2)
        }
    }

    impl Step for SubtractTen {
        type Input = i32;
        type Output = i32;
        fn execute(&self, input: i32) -> Result<i32, PipelineError> {
            Ok(input - 10)
        }
    }

    #[derive(Debug, PartialEq)]
    struct RawUser {
        username: String,
        access_level: u8,
    }
    #[derive(Debug, PartialEq)]
    struct ProcessedUser {
        id: usize,
        display_name: String,
    }

    struct SanitizeName;
    impl Step for SanitizeName {
        type Input = RawUser;
        type Output = String;
        fn execute(&self, input: RawUser) -> Result<String, PipelineError> {
            Ok(input.username.trim().to_lowercase())
        }
    }

    struct CreateProfile;
    impl Step for CreateProfile {
        type Input = String;
        type Output = ProcessedUser;
        fn execute(&self, input: String) -> Result<ProcessedUser, PipelineError> {
            Ok(ProcessedUser {
                id: 101,
                display_name: format!("User: {}", input),
            })
        }
    }

    struct ValidateId;
    impl Step for ValidateId {
        type Input = ProcessedUser;
        type Output = bool;
        fn execute(&self, input: ProcessedUser) -> Result<bool, PipelineError> {
            Ok(input.id > 0)
        }
    }

    #[test]
    fn test_math_pipeline() {
        let pipe = Pipeline::builder::<i32>()
            .add_stage(MultiplyByTwo)
            .add_stage(SubtractTen)
            .build();

        assert_eq!(pipe.run(20).unwrap(), 30);
    }

    #[test]
    fn test_heterogeneous_pipeline_vec() {
        let pipe_a = Pipeline::builder::<i32>()
            .add_stage(MultiplyByTwo)
            .add_stage(SubtractTen)
            .build()
            .into_boxed();

        let pipe_b = Pipeline::builder::<i32>()
            .add_stage(MultiplyByTwo)
            .build()
            .into_boxed();

        let pipeline_registry: Vec<Box<dyn Step<Input = i32, Output = i32>>> = vec![pipe_a, pipe_b];
        let results: Vec<i32> = pipeline_registry
            .iter()
            .map(|p| p.execute(20).unwrap())
            .collect();
        assert_eq!(results, vec![30, 40]);
    }

    #[test]
    fn test_recoverable_error_flow() {
        struct FailStage;
        impl Step for FailStage {
            type Input = i32;
            type Output = i32;
            fn execute(&self, _: i32) -> Result<i32, PipelineError> {
                Err(PipelineError::Recoverable("Temporary glitch".to_string()))
            }
        }

        let pipe = Pipeline::builder::<i32>().add_stage(FailStage).build();
        let result = pipe.run(10);

        match result {
            Err(PipelineError::Recoverable(msg)) => assert_eq!(msg, "Temporary glitch"),
            _ => panic!("Expected a recoverable error"),
        }
    }

    #[test]
    fn test_transformation_chain() {
        let user_pipe = Pipeline::builder::<RawUser>()
            .add_stage(SanitizeName)
            .add_stage(CreateProfile)
            .add_stage(ValidateId)
            .build();

        let input = RawUser {
            username: "  GUEST_USER  ".to_string(),
            access_level: 1,
        };
        assert!(user_pipe.run(input).unwrap());
    }

    #[test]
    fn test_closure_only_pipeline() {
        let pipe = Pipeline::builder::<i32>()
            .add_map(|x| Ok(x + 5))
            .add_map(|x| Ok(x.to_string()))
            .build();

        let result = pipe.run(10).unwrap();
        assert_eq!(result, "15");
    }

    #[test]
    fn test_mixed_struct_and_closure_pipeline() {
        let pipe = Pipeline::builder::<i32>()
            .add_stage(MultiplyByTwo)
            .add_stage(SubtractTen)
            .add_map(|x| {
                if x < 0 {
                    Ok(format!("Negative: {}", x))
                } else {
                    Ok(format!("Positive: {}", x))
                }
            })
            .build();

        assert_eq!(pipe.run(5).unwrap(), "Positive: 0");
        assert_eq!(pipe.run(2).unwrap(), "Negative: -6");
    }
    #[test]
    fn test_retry_logic_success_after_flaking() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // We use Arc/Atomic to track calls across the cloned executions
        struct FlakyStep(Arc<AtomicUsize>);
        impl Step for FlakyStep {
            type Input = i32;
            type Output = i32;
            fn execute(&self, input: i32) -> Result<i32, PipelineError> {
                let attempts = self.0.fetch_add(1, Ordering::SeqCst);
                if attempts < 2 {
                    Err(PipelineError::Recoverable("Flaky".to_string()))
                } else {
                    Ok(input + 1)
                }
            }
        }

        let counter = Arc::new(AtomicUsize::new(0));
        let pipe = Pipeline::builder::<i32>()
            // Retry 2 times means 3 total attempts
            .add_stage(FlakyStep(counter.clone()).with(Retry::times(2)))
            .build();

        let res = pipe.run(10).unwrap();
        assert_eq!(res, 11);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_retry_logic_exhaustion() {
        struct AlwaysFail;
        impl Step for AlwaysFail {
            type Input = i32;
            type Output = i32;
            fn execute(&self, _: i32) -> Result<i32, PipelineError> {
                Err(PipelineError::Recoverable("Persistent Glitch".to_string()))
            }
        }

        let pipe = Pipeline::builder::<i32>()
            .add_stage(AlwaysFail.with(Retry::times(2)))
            .build();

        match pipe.run(10) {
            Err(PipelineError::Recoverable(e)) => assert_eq!(e, "Persistent Glitch"),
            _ => panic!("Expected recoverable error after exhaustion"),
        }
    }

    #[test]
    fn test_retry_logic_stops_on_permanent() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct PermanentFail(Arc<AtomicUsize>);
        impl Step for PermanentFail {
            type Input = i32;
            type Output = i32;
            fn execute(&self, _: i32) -> Result<i32, PipelineError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(PipelineError::Permanent("Fatal".to_string()))
            }
        }

        let counter = Arc::new(AtomicUsize::new(0));
        let pipe = Pipeline::builder::<i32>()
            .add_stage(PermanentFail(counter.clone()).with(Retry::times(10)))
            .build();

        let _ = pipe.run(10);
        // Even with 10 retries, it should stop after the 1st attempt
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn test_policy_order_logger_outside_retry() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let step_counter = Arc::new(AtomicUsize::new(0));
        let logger_counter = Arc::new(AtomicUsize::new(0));

        struct FlakyStep(Arc<AtomicUsize>);
        impl Step for FlakyStep {
            type Input = i32;
            type Output = i32;
            fn execute(&self, input: i32) -> Result<i32, PipelineError> {
                let count = self.0.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(PipelineError::Recoverable("fail".into()))
                } else {
                    Ok(input)
                }
            }
        }

        // 3. Define a Mock Logger Policy
        struct MockLogger(Arc<AtomicUsize>);
        impl<S: Step> Policy<S> for MockLogger {
            type Decorated = MockLoggerStep<S>;
            fn apply(self, step: S) -> Self::Decorated {
                MockLoggerStep {
                    inner: step,
                    counter: self.0,
                }
            }
        }
        struct MockLoggerStep<S> {
            inner: S,
            counter: Arc<AtomicUsize>,
        }
        impl<S: Step> Step for MockLoggerStep<S> {
            type Input = S::Input;
            type Output = S::Output;
            fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
                self.counter.fetch_add(1, Ordering::SeqCst);
                self.inner.execute(input)
            }
        }

        // TEST: [Logger [Retry [Step]]]
        // The logger wraps the entire Retry block.
        let pipe = Pipeline::builder::<i32>()
            .add_stage(
                FlakyStep(step_counter.clone())
                    .with(Retry::times(2)) // Inner layer
                    .with(MockLogger(logger_counter.clone())), // Outer layer
            )
            .build();

        let res = pipe.run(10).unwrap();

        // The step was attempted 3 times (2 fails + 1 success)
        assert_eq!(step_counter.load(Ordering::SeqCst), 3);
        // The Logger was only called ONCE because it sits outside the retry loop
        assert_eq!(logger_counter.load(Ordering::SeqCst), 1);
        assert_eq!(res, 10);
    }

    #[test]
    fn test_policy_order_logger_inside_retry() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let step_counter = Arc::new(AtomicUsize::new(0));
        let logger_counter = Arc::new(AtomicUsize::new(0));

        struct FlakyStep(Arc<AtomicUsize>);
        impl Step for FlakyStep {
            type Input = i32;
            type Output = i32;
            fn execute(&self, input: i32) -> Result<i32, PipelineError> {
                let count = self.0.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(PipelineError::Recoverable("fail".into()))
                } else {
                    Ok(input)
                }
            }
        }

        struct MockLogger(Arc<AtomicUsize>);
        impl<S: Step> Policy<S> for MockLogger {
            type Decorated = MockLoggerStep<S>;
            fn apply(self, step: S) -> Self::Decorated {
                MockLoggerStep {
                    inner: step,
                    counter: self.0,
                }
            }
        }
        struct MockLoggerStep<S> {
            inner: S,
            counter: Arc<AtomicUsize>,
        }
        impl<S: Step> Step for MockLoggerStep<S> {
            type Input = S::Input;
            type Output = S::Output;
            fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
                self.counter.fetch_add(1, Ordering::SeqCst);
                self.inner.execute(input)
            }
        }

        // TEST: [Retry [Logger [Step]]]
        // The Retry loop wraps the Logger.
        let pipe = Pipeline::builder::<i32>()
            .add_stage(
                FlakyStep(step_counter.clone())
                    .with(MockLogger(logger_counter.clone())) // Inner layer
                    .with(Retry::times(2)), // Outer layer
            )
            .build();

        let res = pipe.run(10).unwrap();

        // The step was attempted 3 times
        assert_eq!(step_counter.load(Ordering::SeqCst), 3);
        // The Logger was also called 3 times because it is INSIDE the retry loop
        assert_eq!(logger_counter.load(Ordering::SeqCst), 3);
        assert_eq!(res, 10);
    }
}
