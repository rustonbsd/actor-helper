
use std::io;

use actor_helper::{act, act_ok, block_on, spawn_actor_blocking, ActorSync, Handle, Receiver};

// Public API
pub struct Counter {
    handle: Handle<CounterActor, io::Error>,
}

impl Counter {
    pub fn new() -> Self {
        let (handle, rx) = Handle::channel();

        let _join_handle = spawn_actor_blocking(CounterActor { value: 0, rx });

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

    pub fn set_positive(&self, value: i32) -> io::Result<()> {
        self.handle.call_blocking(act!(actor => async move {
            if value <= 0 {
                Err(io::Error::new(io::ErrorKind::Other, "Value must be positive"))
            } else {
                actor.value = value;
                Ok(())
            }
        }))
    }
}

// Private actor implementation
struct CounterActor {
    value: i32,
    rx: Receiver<actor_helper::Action<CounterActor>>,
}

impl ActorSync<io::Error> for CounterActor {
    fn run_blocking(&mut self) -> Result<(), io::Error> {
        loop {
            if let Ok(action) = self.rx.recv() {
                block_on(action(self));
            }
        }
    }
}

pub fn main() -> io::Result<()> {
    let counter = Counter::new();

    counter.increment(5)?;
    println!("Value: {}", counter.get()?);

    counter.set_positive(10)?;
    println!("Value: {}", counter.get()?);

    Ok(())
}