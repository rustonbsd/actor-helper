use std::{io, sync::mpsc};

use actor_helper::{Actor, Handle, act_ok, act};
use futures_executor::block_on;

// Public API
pub struct Counter {
    handle: Handle<CounterActor>,
    _join_handle: std::thread::JoinHandle<()>,
}

impl Counter {
    pub fn new() -> Self {
        let (handle, rx) = Handle::channel();
        let actor = CounterActor { value: 0, rx };

        let join_handle = std::thread::spawn(move || {
            let mut actor = actor;
            let _ = block_on(actor.run());
        });

        Self { handle, _join_handle: join_handle }
    }

    pub fn increment(&self, by: i32) -> io::Result<()> {
        self.handle.call(act_ok!(actor => async move {
            actor.value += by;
        }))
    }

    pub fn get(&self) -> io::Result<i32> {
        self.handle.call(act_ok!(actor => async move { 
            actor.value
        }))
    }

    pub fn set_positive(&self, value: i32) -> io::Result<()> {
        self.handle.call(act!(actor => async move {
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
    rx: mpsc::Receiver<actor_helper::Action<CounterActor>>,
}

impl Actor for CounterActor {
    async fn run(&mut self) -> io::Result<()> {
        loop {
            if let Ok(action) = self.rx.try_recv() {
                action(self).await;
            }
        }
    }
}

fn main() -> io::Result<()> {
    let counter = Counter::new();
    
    counter.increment(5)?;
    println!("Value: {}", counter.get()?);
    
    counter.set_positive(10)?;
    println!("Value: {}", counter.get()?);
    
    Ok(())
}