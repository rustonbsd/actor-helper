# actor-helper

[![Crates.io](https://img.shields.io/crates/v/actor-helper.svg)](https://crates.io/crates/actor-helper)
[![Documentation](https://docs.rs/actor-helper/badge.svg)](https://docs.rs/actor-helper)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A minimal, opinionated actor framework for Rust.

## Features

- **Runtime agnostic**: Works with `tokio`, `async-std`, or blocking threads
- **Dynamic error types**: Use `io::Error`, `anyhow::Error`, `String`, or custom types
- **Simple API**: Create actors with just a handle and receiver
- **Type-safe**: Compile-time guarantees for actor interactions
- **Panic-safe**: Automatic panic capture and error propagation with location tracking
- **Ergonomic macros**: `act!` and `act_ok!` for writing actor actions
- **Thread-safe**: Clone handles to communicate from anywhere

## Direct Actor Access

`actor-helper` provides **direct mutable access** to actor state through closures. Instead of defining message types and handlers, you write functions that directly manipulate the actor:

```rust
// Traditional message passing approach:
// actor.send(Increment(5)).await?;

// actor-helper approach - direct function execution:
handle.call(act_ok!(actor => async move { actor.value += 5; })).await?;
```

This design offers several advantages:
- **No message types**: Write functions directly instead of defining enums/structs
- **Type safety**: Full compile-time checking of actor interactions
- **Flexibility**: Execute any logic on the actor state, including async operations
- **Simplicity**: Less boilerplate, more readable code

The `Handle` is cloneable and can be shared across threads, but all access to the actor's mutable state is serialized through the actor's mailbox, maintaining single-threaded safety.

## Important: Keep Actions Fast

**Actions run sequentially and should complete quickly.** A slow action blocks the entire actor:

```rust
// DON'T: Long-running work blocks the actor
pub async fn process(&self) -> io::Result<()> {
    self.handle.call(act!(actor => async move {
        tokio::time::sleep(Duration::from_secs(10)).await;  // Blocks everything!
        Ok(())
    })).await
}

// DO: Get state, process outside, write back
pub async fn process(&self) -> io::Result<()> {
    let data = self.handle.call(act_ok!(actor => async move {
        actor.data.clone()
    })).await?;
    
    // Slow work happens outside
    let new_data = expensive_computation(&data).await;
    
    self.handle.call(act_ok!(actor => async move {
        actor.data = new_data;
    })).await
}

// DO: Quick mutations inside
pub async fn increment(&self) -> io::Result<()> {
    self.handle.call(act_ok!(actor => async move {
        actor.value += 1;  // Fast
    })).await
}
```

For background tasks, use `tokio::select!` in the actor's `run()` method

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
actor-helper = { version = "0.2", features = ["tokio"] }
tokio = { version = "1", features = ["rt-multi-thread"] }
```

## Example with tokio

```rust
use std::io;
use actor_helper::{Actor, Handle, Receiver, act, act_ok, spawn_actor};

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
                Err(io::Error::new(io::ErrorKind::Other, "Value must be positive"))
            } else {
                actor.value = value;
                Ok(())
            }
        })).await
    }
}

// Private actor implementation
struct CounterActor {
    value: i32,
    rx: Receiver<actor_helper::Action<CounterActor>>,
}

impl Actor<io::Error> for CounterActor {
    async fn run(&mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => {
                    action(self).await;
                }
                // Your background tasks here!
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

## Blocking/Sync Example

No async runtime required:

```rust
use std::io;
use actor_helper::{ActorSync, Handle, Receiver, act_ok, spawn_actor_blocking, block_on};

pub struct Counter {
    handle: Handle<CounterActor, io::Error>,
}

impl Counter {
    pub fn new() -> Self {
        let (handle, rx) = Handle::channel();
        spawn_actor_blocking(CounterActor { value: 0, rx });
        Self { handle }
    }

    pub fn increment(&self, by: i32) -> io::Result<()> {
        self.handle.call_blocking(act_ok!(actor => async move {
            actor.value += by;
        }))
    }

    pub fn get(&self) -> io::Result<i32> {
        self.handle.call_blocking(act_ok!(actor => async move {
            actor.value
        }))
    }
}

struct CounterActor {
    value: i32,
    rx: Receiver<actor_helper::Action<CounterActor>>,
}

impl ActorSync<io::Error> for CounterActor {
    fn run_blocking(&mut self) -> io::Result<()> {
        loop {
            if let Ok(action) = self.rx.recv() {
                block_on(action(self));
            }
        }
    }
}

fn main() -> io::Result<()> {
    let counter = Counter::new();
    counter.increment(5)?;
    println!("Value: {}", counter.get()?);
    Ok(())
}
```

## Using Custom Error Types

### With `anyhow::Error`

Enable the feature:

```toml
[dependencies]
actor-helper = { version = "0.2", features = ["anyhow", "tokio"] }
anyhow = "1"
```

Then use it in your code:

```rust
use anyhow::{anyhow, Result};
use actor_helper::{Actor, Handle, Receiver, act, spawn_actor};

pub struct Counter {
    handle: Handle<CounterActor, anyhow::Error>,
}

impl Counter {
    pub async fn set_positive(&self, value: i32) -> Result<()> {
        self.handle.call(act!(actor => async move {
            if value <= 0 {
                Err(anyhow!("Value must be positive"))
            } else {
                actor.value = value;
                Ok(())
            }
        })).await
    }
}

struct CounterActor {
    value: i32,
    rx: Receiver<actor_helper::Action<CounterActor>>,
}

impl Actor<anyhow::Error> for CounterActor {
    async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => action(self).await,
            }
        }
    }
}
```

### Custom Error Type

Implement the `ActorError` trait:

```rust
use actor_helper::ActorError;

#[derive(Debug)]
enum MyError {
    ActorPanic(String),
    // ... your error variants
}

impl ActorError for MyError {
    fn from_actor_message(msg: String) -> Self {
        MyError::ActorPanic(msg)
    }
}

// Now use Handle<MyActor, MyError>
```

## async-std Support

```toml
[dependencies]
actor-helper = { version = "0.2", features = ["async-std"] }
async-std = { version = "1", features = ["attributes"] }
```

The API is identical to tokio, just use `#[async_std::main]` instead.

## How It Works

1. **Create a handle and receiver**: `Handle::channel()` creates an unbounded channel pair
2. **Spawn the actor**: Pass the receiver to your actor and spawn it with `spawn_actor()` or `spawn_actor_blocking()`
3. **Call actions**: Use `handle.call()` or `handle.call_blocking()` with `act!` or `act_ok!` macros
4. **Sequential execution**: Actions are processed one at a time by the actor
5. **Panic safety**: Panics are caught and converted to errors with the call site location

The unbounded channel helps prevent one specific deadlock scenario where an actor needs to call itself, **but it doesn't prevent all possible deadlock situations**.

## License

MIT
