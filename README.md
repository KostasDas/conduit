# Conduit

`conduit` is a type-safe pipeline engine for Rust, designed for high-performance structured data transformation. 

It utilizes recursive type composition to ensure the transformation chain is validated at compile-time
If it compiles, the types fit, and the data flows—with **zero runtime overhead**.

---

## Core Philosophy

* **Static over Dynamic:** No `Box<dyn Step>` or trait objects are used in the core pipeline. Everything is resolved at compile-time.
* **Pipeline Steps:**. Define your pipeline steps by implementing the `Step` trait. It is also possible to just use a closure for simple tasks.
* **Decorator Pattern for custom Policies:** Define your own data transformation policies by implementing the `Policy` trait. Retrying, timing out, logging, anything is possible. This keeps your business logic (the `Step`) pure and separated from infrastructure concerns. Note that policies are not available for closures, only for steps.
By default, `Conduit` comes with just one policy: retries. It is meant to serve as an example.
* **Inference-First:** Once you define your initial input type, the rest of the pipeline infers its types automatically. No more redundant type annotations.

---

## Getting Started

### 1. Define a Step
A `Step` is a discrete unit of transformation. It defines what it takes in and what it produces.

```rust
use conduit::{Step, PipelineError};

struct MultiplyByTwo;

impl Step for MultiplyByTwo {
    type Input = i32;
    type Output = i32;

    fn execute(&self, input: i32) -> Result<i32, PipelineError> {
        Ok(input * 2)
    }
}
```

### 2. Build and Run a Pipeline
Use the fluent builder API to compose your steps. By specifying the type at `builder::<I>()`, all subsequent closures and stages benefit from full type inference.

```rust
let pipe = Pipeline::builder::<i32>() // Define the entry type
    .add_stage(MultiplyByTwo)          // Add a struct-based step
    .add_map(|x| Ok(x - 5))            // Add a lightweight closure (x is inferred as i32)
    .build();

let result = pipe.run(20).unwrap(); 
assert_eq!(result, 35); // (20 * 2) - 5
```

---

## Advanced: Policies & Reliability

Conduit allows you to "decorate" any concrete `Step` with a `Policy` using the `.with()` method. This wraps the step in new logic without changing its Input/Output contract.

### The Retry Policy
The `Retry` policy intercepts `Recoverable` errors and re-executes the step.

```rust
let pipe = Pipeline::builder::<RawUser>()
    .add_stage(DatabaseLookup.with(Retry::times(3))) // Retry up to 3 times
    .add_map(|user| Ok(user.id))
    .build();
```

> **⚠️ Note on Policy Order:** Policies are applied like layers of an onion. 
> * `step.with(Retry::times(3)).with(Logger::new())` will log **every single retry attempt**. 
> * `step.with(Logger::new()).with(Retry::times(3))` will only log the **final result** of the retry loop.

---

## Extending Conduit

### Implementing a Custom Policy
To create a new behavior (like a `Logger`), you must implement the `Policy` trait and provide a `Step` wrapper.

```rust
pub struct Logger;

impl<S: Step> Policy<S> for Logger {
    type Decorated = LoggingStep<S>;

    fn apply(self, step: S) -> Self::Decorated {
        LoggingStep { inner: step }
    }
}

// Internal wrapper that implements Step
pub struct LoggingStep<S> { 
    inner: S 
}

impl<S: Step> Step for LoggingStep<S> {
    type Input = S::Input;
    type Output = S::Output;

    fn execute(&self, input: Self::Input) -> Result<Self::Output, PipelineError> {
        println!("Stage transition started...");
        let result = self.inner.execute(input);
        println!("Stage transition finished.");
        result
    }
}
```

---

## Architectural Details

### Error Handling
Conduit distinguishes between two types of failures:
* **`PipelineError::Recoverable`**: Signals that a policy (like `Retry`) should attempt to fix the issue.
* **`PipelineError::Permanent`**: Signals a fatal flaw (like validation failure). The pipeline stops immediately, bypassing all retry logic.

### Zero-Cost Abstractions
Because Conduit uses generics and recursive structures, the "cost" of adding a stage is purely a compile-time cost. At runtime, there is no performance difference between a Conduit pipeline and a manually written sequence of function calls.


### Type Erasure and Collections

Pipelines use complex, nested generic types (e.g., `PipelineStep<NoOp<i32>, RetryStep<MultiplyByTwo>>`). While this is great for performance, it makes it difficult to store different pipelines in a single collection. Conduit provides two ways to handle this.

#### 1. The `into_boxed` Method
If your pipelines share the same **Input** and **Output** types, you can erase the internal structure using `into_boxed`. This returns a `Box<dyn Step<Input = I, Output = O>>`.

```rust
let pipe_a = Pipeline::builder::<i32>().add_stage(MultiplyByTwo).build().into_boxed();
let pipe_b = Pipeline::builder::<i32>().add_map(|x| Ok(x + 1)).build().into_boxed();

// Now they can live in the same Vec
let inventory: Vec<Box<dyn Step<Input = i32, Output = i32>>> = vec![pipe_a, pipe_b];
```

#### 2. The Enum Wrapper (Static Dispatch)
If you want to bypass the need for trait objects entirely or handle pipelines with different signatures, use an Enum Wrapper. This is the "Conduit Way" to maintain high performance while grouping diverse pipelines.

```rust
/// A collection of known transformation pipelines
enum UserProcessingTask {
    Registration(Pipeline<PipelineStep<NoOp<RawUser>, SanitizeName>>),
    Deletion(Pipeline<PipelineStep<NoOp<UserId>, DatabaseDeleteStep>>),
}

impl UserProcessingTask {
    // You can implement a uniform execution interface
    fn run_task(self, input: UserInput) -> Result<(), PipelineError> {
        match self {
            Self::Registration(p) => {
                let user = input.into_raw();
                p.run(user).map(|_| ())
            },
            Self::Deletion(p) => {
                let id = input.into_id();
                p.run(id).map(|_| ())
            }
        }
    }
}
```


### Shameless plug: Conduit + Kraquen
**Conduit** pairs perfectly with **[Kraquen](https://github.com/KostasDas/kraquen)**—our thread-safe, generic queue. While Conduit defines **how** data is transformed, Kraquen handles **when** and **where** it is processed.

### The "Factory Line" Demo
Use Kraquen to pass data between threads and Conduit to perform the work at each stage (this is not meant to be an executable example, just a demo)
```rust
#[test]
fn test_conduit_kraquen_flow() {
    use kraquen::{Queue, QueueMode};
    use conduit::prelude::*;
    
    let intake = Queue::new(QueueMode::FIFO);
    let outbox = Queue::new(QueueMode::FIFO);

    let worker_pipeline = Pipeline::builder::<RawWork>()
        .add_map(|work| Ok(FinishedWork { 
            id: work.id, 
            checksum: work.payload.len() 
        }))
        .build();

    intake.push(RawWork { id: 1, payload: "Hello".to_string() });

    if let Some(work) = intake.pop() {
        let processed = worker_pipeline.run(work).unwrap();
        outbox.push(processed);
    }

    assert_eq!(outbox.pop().unwrap().checksum, 5);
}
```
