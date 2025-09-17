use actor_helper::{Actor, Handle, act_ok, act};
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
        self.handle.call(act_ok!(actor => async move {
            actor.value += by;
        })).await
    }

    pub async fn get(&self) -> Result<i32> {
        self.handle.call(act_ok!(actor => async move { 
            actor.value
        })).await
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
        loop {
            tokio::select! {
                Some(action) = self.rx.recv() => {
                    action(self).await;
                }
                // Your background reader.recv() etc here!
            }
        }
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