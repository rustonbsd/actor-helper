# actor-helper

[![Crates.io](https://img.shields.io/crates/v/actor-helper.svg)](https://crates.io/crates/actor-helper)
[![Documentation](https://docs.rs/actor-helper/badge.svg)](https://docs.rs/actor-helper)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A minimal, ergonomic actor runtime built on top of Tokio.



## Features

- **Simple API**: Create actors with just a handle and receiver
- **Type-safe**: Compile-time guarantees for actor interactions
- **Panic-safe**: Automatic panic capture and error propagation
- **Ergonomic macros**: `act!` and `act_ok!` for writing actor actions
- **Thread-safe**: Clone handles to communicate from anywhere

## Direct Actor Access

`actor-helper` provides **direct mutable access** to actor state through closures. Instead of defining message types and handlers, you write functions that directly manipulate the actor:

```rust
// Traditional message passing approach:
// actor.send(Increment(5)).await?;

// actor-helper approach - direct function execution:
handle.call(act_ok!(actor => { actor.value += 5; })).await?;
```

This design offers several advantages:
- **No message types**: Write functions directly instead of defining enums/structs
- **Type safety**: Full compile-time checking of actor interactions
- **Flexibility**: Execute any logic on the actor state, including async operations
- **Simplicity**: Less boilerplate, more readable code

The `Handle` is clonable and can be shared across threads, but all access to the actor's mutable state is serialized through the actor's mailbox, maintaining single-threaded safety.

## ⚠️ Important: Keep Actions Fast

**Actions passed to `handle.call()` should complete quickly.** The actor processes actions sequentially, so a slow action blocks the entire mailbox:

```rust
// ❌ DON'T: Long-running operations block the actor
handle.call(act!(actor => async move {
    tokio::time::sleep(Duration::from_secs(10)).await; // Blocks other actions!
    Ok(())
})).await?;

// ✅ DO: Spawn long-running work separately
handle.call(act_ok!(actor => {
    let data = actor.get_work_data();
    tokio::spawn(async move {
        // Long operation runs independently
        process_data(data).await;
    });
})).await?;
```

For background work or continuous tasks, implement them in the actor's `run()` method using `tokio::select!`.

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
actor-helper = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "sync"] }
anyhow = "1"
```

## Example

```rust
use actor_helper::{Actor, Handle, act_ok!, act!};
use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

// Public API
pub struct Counter {
    handle: Handle<CounterActor>,
}

impl Counter {
    pub fn new() -> Self {
        let (handle, rx) = Handle::channel(128);
        let actor = CounterActor { value: 0, rx };
        
        tokio::spawn(async move {
            let mut actor = actor;
            let _ = actor.run().await;
        });

        Self { handle }
    }

    pub async fn increment(&self, by: i32) -> Result<()> {
        self.handle.call(act_ok!(actor => {
            actor.value += by;
        })).await
    }

    pub async fn get(&self) -> Result<i32> {
        self.handle.call(act_ok!(actor => actor.value)).await
    }

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

// Private actor implementation
struct CounterActor {
    value: i32,
    rx: mpsc::Receiver<actor_helper::Action<CounterActor>>,
}

impl Actor for CounterActor {
    async fn run(&mut self) -> Result<()> {
        while let Some(action) = self.rx.recv().await {
            action(self).await;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let counter = Counter::new();
    
    counter.increment(5).await?;
    println!("Value: {}", counter.get().await?);
    
    counter.set_positive(10).await?;
    println!("Value: {}", counter.get().await?);
    
    Ok(())
}
```

## License

MIT