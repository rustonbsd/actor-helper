use actor_helper::{Actor, ActorState, Handle, Receiver, act, act_ok};
use anyhow::{Result, anyhow};

// Public API
pub struct Counter {
    handle: Handle<CounterActor, anyhow::Error>,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            handle: Handle::spawn(|rx| CounterActor { value: 0, rx }).0,
        }
    }
}

impl Counter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn increment(&self, by: i32) -> Result<()> {
        self.handle
            .call(act_ok!(actor => async move {
                actor.value += by;
            }))
            .await
    }

    pub async fn get(&self) -> Result<i32> {
        self.handle
            .call(act_ok!(actor => async move {
                actor.value
            }))
            .await
    }

    pub async fn set_positive(&self, value: i32) -> Result<()> {
        self.handle
            .call(act!(actor => async move {
                if value <= 0 {
                    Err(anyhow!("Value must be positive"))
                } else {
                    actor.value = value;
                    Ok(())
                }
            }))
            .await
    }

    pub async fn is_running(&self) -> bool {
        self.handle.state() == ActorState::Running
    }
}

// Private actor implementation
struct CounterActor {
    value: i32,
    rx: Receiver<actor_helper::Action<CounterActor>>,
}

impl Actor<anyhow::Error> for CounterActor {
    async fn run(&mut self) -> Result<()> {
        loop {
            if let Ok(action) = self.rx.recv_async().await {
                action(self).await;
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

    println!("Is running: {}", counter.is_running().await);

    Ok(())
}
