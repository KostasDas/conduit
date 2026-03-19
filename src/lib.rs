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

/// A wrapper for anonymous closures to implement the [`Step`] trait.
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
}
