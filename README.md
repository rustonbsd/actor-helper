# actor-helper

[![Crates.io](https://img.shields.io/crates/v/actor-helper.svg)](https://crates.io/crates/actor-helper)
[![Documentation](https://docs.rs/actor-helper/badge.svg)](https://docs.rs/actor-helper)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Lightweight, runtime-agnostic actor pattern with dynamic error types.

## Features

- **Runtime agnostic**: Works with `tokio`, `async-std`, or blocking threads
- **Dynamic errors**: Use `io::Error`, `anyhow::Error`, or custom error types
- **Type-safe**: Compile-time guarantees for actor interactions
- **Panic-safe**: Panics caught and converted to errors with location info
- **Ergonomic**: `act!` and `act_ok!` macros for inline actions
- **Thread-safe**: Clone handles to communicate from anywhere

## Direct Actor Access

Write closures that directly access actor state—no message types needed:

```rust
// Direct function execution
handle.call(act_ok!(actor => async move { 
    actor.value += 5; 
})).await?;
```

**Keep actions fast.** Long operations block the mailbox:

```rust
// ❌ DON'T: Blocks other actions
handle.call(act!(actor => async move {
    tokio::time::sleep(Duration::from_secs(10)).await;
    Ok(())
})).await?;

// ✅ DO: Spawn long work separately
handle.call(act_ok!(actor => {
    let data = actor.get_work_data();
    tokio::spawn(async move { process_data(data).await });
})).await?;
```

## Quick Start

```toml
[dependencies]
actor-helper = "0.2"
tokio = { version = "1", features = ["rt-multi-thread"] }
```

## Example

```rust
use std::io;
use actor_helper::{Actor, Handle, Receiver, act_ok, act, spawn_actor};

// Public API
pub struct Counter {
    handle: Handle<CounterActor, io::Error>,
}

impl Counter {
    pub fn new() -> Self {
        let (handle, rx) = Handle::channel();
        spawn_actor(CounterActor { value: 0, rx });
        Self { handle }
    }

    pub async fn increment(&self, by: i32) -> io::Result<()> {
        self.handle.call(act_ok!(actor => async move {
            actor.value += by;
        })).await
    }

    pub async fn get(&self) -> io::Result<i32> {
        self.handle.call(act_ok!(actor => async move { 
            actor.value
        })).await
    }

    pub async fn set_positive(&self, value: i32) -> io::Result<()> {
        self.handle.call(act!(actor => async move {
            if value <= 0 {
                Err(io::Error::new(io::ErrorKind::Other, "must be positive"))
            } else {
                actor.value = value;
                Ok(())
            }
        })).await
    }
}

// Private actor
struct CounterActor {
    value: i32,
    rx: Receiver<actor_helper::Action<CounterActor>>,
}

impl Actor<io::Error> for CounterActor {
    async fn run(&mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => action(self).await,
            }
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let counter = Counter::new();
    
    counter.increment(5).await?;
    println!("Value: {}", counter.get().await?);
    
    counter.set_positive(10).await?;
    println!("Value: {}", counter.get().await?);
    
    Ok(())
}
```

## Using `anyhow::Error`

Enable the feature and use `anyhow::Error` as your error type:

```toml
[dependencies]
actor-helper = { version = "0.2", features = ["anyhow"] }
```

```rust
use anyhow::{anyhow, Result};
use actor_helper::{Actor, Handle, Receiver};

pub struct Counter {
    handle: Handle<CounterActor, anyhow::Error>,
}

impl Actor<anyhow::Error> for CounterActor {
    async fn run(&mut self) -> Result<()> {
        // ...
    }
}
```

## License

MIT
