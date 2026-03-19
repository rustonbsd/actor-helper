use std::io;

use actor_helper::{Actor, Handle, Receiver, act, act_ok};

// Public API
pub struct Counter {
    handle: Handle<CounterActor, io::Error>,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            handle: Handle::spawn(|rx| CounterActor { value: 0, rx }).0,
        }
    }
}

impl Counter {

    pub async fn increment(&self, by: i32) -> io::Result<()> {
        self.handle.call(act_ok!(actor => async move {
            actor.value += by;
        })).await
    }

    pub async  fn get(&self) -> io::Result<i32> {
        self.handle.call(act_ok!(actor => async move {
            actor.value
        })).await
    }

    pub async  fn set_positive(&self, value: i32) -> io::Result<()> {
        self.handle.call(act!(actor => async move {
            if value <= 0 {
                Err(io::Error::other("Value must be positive"))
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
            if let Ok(action) = self.rx.recv_async().await {
                action(self).await;
            }
        }
    }
}

#[async_std::main]
async fn main() -> io::Result<()> {
    let counter = Counter::default();

    counter.increment(5).await?;
    println!("Value: {}", counter.get().await?);

    counter.set_positive(10).await?;
    println!("Value: {}", counter.get().await?);

    Ok(())
}
