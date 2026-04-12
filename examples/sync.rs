use std::io;

use actor_helper::{Handle, act, act_ok};

// Public API
pub struct Counter {
    handle: Handle<CounterActor, io::Error>,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            handle: Handle::spawn_blocking(CounterActor {
                value: 0,
            })
            .0,
        }
    }
}

impl Counter {
    pub fn new() -> Self {
        Self::default()
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

    pub fn set_positive(&self, value: i32) -> io::Result<()> {
        self.handle.call_blocking(act!(actor => async move {
            if value <= 0 {
                Err(io::Error::other("Value must be positive"))
            } else {
                actor.value = value;
                Ok(())
            }
        }))
    }

    pub fn stop(&self) {
        self.handle.shutdown();
        self.handle.wait_stopped_blocking();
    }
}

// Private actor implementation
struct CounterActor {
    value: i32,
}

pub fn main() -> io::Result<()> {
    let counter = Counter::new();

    counter.increment(5)?;
    println!("Value: {}", counter.get()?);

    counter.set_positive(10)?;
    println!("Value: {}", counter.get()?);

    counter.stop();

    assert_eq!(counter.handle.state(), actor_helper::ActorState::Stopped);

    Ok(())
}
