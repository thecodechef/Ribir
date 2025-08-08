# tokio-run-until-stalled
## Overview
A utility library for running Tokio's LocalSet asynchronous tasks until no more progress can be made on any task in the LocalSet.

## Usage
To use this library, add the following dependency to your Cargo.toml file:
``` toml
[dependencies]
tokio-run-until-stalled = "0.1.0"
```
Then, import the library in your Rust file:
``` rust
use tokio_run_until_stalled::*;
```

### Examples
Running a LocalSet until it stalls
``` rust
use std::time::Duration;
use tokio::{runtime::Builder, time::sleep, task::LocalSet};
use tokio_run_until_stalled::*;

fn main() {
    let local_set = LocalSet::new();
    local_set.spawn_local(async {
        println!("Task 1 completed");
    });
    local_set.spawn_local(async {
        sleep(Duration::from_millis(100)).await;
        println!("Task 2 completed");
    });
    let rt = Builder::new_multi_thread().enable_all().build().unwrap();
    let result = rt.run_until_stalled(local_set);
    assert!(result.is_some());
    rt.block_on(async { sleep(Duration::from_millis(150)).await; });
    let final_result = rt.run_until_stalled(result.unwrap());
    assert!(final_result.is_none());
}
```
Converting a LocalSet to RunUntilStalled
``` rust
use tokio::{task::LocalSet};
use tokio_run_until_stalled::*;

#[tokio::main]
fn main() {
    let local_set = LocalSet::new();
    local_set.spawn_local(async move {
       // ...
    });
    // ... spawn more task.
    let mut run_until_stalled = local_set.run_until_stalled();
    while !run_until_stalled.is_done {
        run_until_stalled = run_until_stalled.await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```
## API Documentation
Please refer to the API documentation for more information on the library's API.

License
This library is licensed under the MIT License.

I hope this helps! Let me know if you need any further assistance.