#[derive(Debug)]
pub enum PipelineError {
    Recoverable(String),
    Permanent(String),
}

pub trait Stage {
    type Input;
    type Output;

    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError>;
}

pub struct Pipeline<S> {
    steps: S,
}

impl<S> Pipeline<S>
where
    S: Stage,
{
    pub fn builder(stage: S) -> PipelineBuilder<S> {
        PipelineBuilder { start_node: stage }
    }

    pub fn run(&self, input: S::Input) -> Result<S::Output, PipelineError> {
        self.steps.execute(input)
    }
}

impl<S> Pipeline<S>
where
    S: Stage + 'static,
{
    pub fn into_boxed(self) -> Box<dyn Stage<Input = S::Input, Output = S::Output>> {
        Box::new(self.steps)
    }
}

pub struct PipelineBuilder<S> {
    start_node: S,
}

impl<S> PipelineBuilder<S>
where
    S: Stage,
{
    pub fn new(stage: S) -> Self {
        Self { start_node: stage }
    }

    pub fn add_stage<A>(self, action: A) -> PipelineBuilder<PipelineStep<S, A>>
    where
        A: Stage<Input = S::Output>,
    {
        let step = PipelineStep {
            current_step: self.start_node,
            next_step: action,
        };
        PipelineBuilder { start_node: step }
    }

    pub fn build(self) -> Pipeline<impl Stage<Input = S::Input, Output = S::Output>> {
        let final_chain = PipelineStep {
            current_step: self.start_node,
            next_step: End::new(),
        };

        Pipeline { steps: final_chain }
    }
}

#[doc(hidden)]
pub struct PipelineStep<Current, Next> {
    current_step: Current,
    next_step: Next,
}

impl<Current, Next> Stage for PipelineStep<Current, Next>
where
    Current: Stage,
    Next: Stage<Input = Current::Output>,
{
    type Input = Current::Input;
    type Output = Next::Output;
    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
        let res = self.current_step.execute(input)?;
        self.next_step.execute(res)
    }
}

#[doc(hidden)]
pub struct End<T> {
    _marker: std::marker::PhantomData<T>,
}

impl<T> End<T> {
    fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> Stage for End<T> {
    type Input = T;
    type Output = T;

    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Math Test Helpers ---
    struct MultiplyByTwo;
    struct SubtractTen;

    impl Stage for MultiplyByTwo {
        type Input = i32;
        type Output = i32;
        fn execute(&self, input: i32) -> Result<i32, PipelineError> {
            Ok(input * 2)
        }
    }

    impl Stage for SubtractTen {
        type Input = i32;
        type Output = i32;
        fn execute(&self, input: i32) -> Result<i32, PipelineError> {
            Ok(input - 10)
        }
    }

    // --- Complex Type Test Helpers ---
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
    impl Stage for SanitizeName {
        type Input = RawUser;
        type Output = String;
        fn execute(&self, input: RawUser) -> Result<String, PipelineError> {
            Ok(input.username.trim().to_lowercase())
        }
    }

    struct CreateProfile;
    impl Stage for CreateProfile {
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
    impl Stage for ValidateId {
        type Input = ProcessedUser;
        type Output = bool;
        fn execute(&self, input: ProcessedUser) -> Result<bool, PipelineError> {
            Ok(input.id > 0)
        }
    }

    // --- Tests ---

    #[test]
    fn test_math_pipeline() {
        let pipe = Pipeline::builder(MultiplyByTwo)
            .add_stage(SubtractTen)
            .build();

        let result = pipe.run(20).unwrap();
        assert_eq!(result, 30);
    }

    #[test]
    fn test_heterogeneous_pipeline_vec() {
        let pipe_a = Pipeline::builder(MultiplyByTwo)
            .add_stage(SubtractTen)
            .build()
            .into_boxed();

        let pipe_b = Pipeline::builder(MultiplyByTwo).build().into_boxed();

        let pipeline_registry: Vec<Box<dyn Stage<Input = i32, Output = i32>>> =
            vec![pipe_a, pipe_b];

        assert_eq!(pipeline_registry.len(), 2);

        let input = 20;
        let results: Vec<i32> = pipeline_registry
            .iter()
            .map(|pipe| pipe.execute(input).unwrap())
            .collect();

        assert_eq!(results, vec![30, 40]);
    }

    #[test]
    fn test_recoverable_error_flow() {
        struct FailStage;
        impl Stage for FailStage {
            type Input = i32;
            type Output = i32;
            fn execute(&self, _: i32) -> Result<i32, PipelineError> {
                Err(PipelineError::Recoverable("Temporary glitch".to_string()))
            }
        }

        let pipe = Pipeline::builder(FailStage).build();
        let result = pipe.run(10);

        match result {
            Err(PipelineError::Recoverable(msg)) => assert_eq!(msg, "Temporary glitch"),
            _ => panic!("Expected a recoverable error"),
        }
    }

    #[test]
    fn test_transformation_chain() {
        let user_pipe = Pipeline::builder(SanitizeName)
            .add_stage(CreateProfile)
            .add_stage(ValidateId)
            .build();

        let input = RawUser {
            username: "  GUEST_USER  ".to_string(),
            access_level: 1,
        };

        let result = user_pipe.run(input).unwrap();
        assert!(result);
    }
}
