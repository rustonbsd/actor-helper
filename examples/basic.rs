use actor_helper::{Actor, Handle, Receiver, act, act_ok, spawn_actor};
use anyhow::{Result, anyhow};

// Public API
pub struct Counter {
    handle: Handle<CounterActor, anyhow::Error>,
}

impl Default for Counter {
    fn default() -> Self {
        let (handle, rx) = Handle::channel();
        let _join_handle = spawn_actor(CounterActor { value: 0, rx });

        Self { handle }
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

    Ok(())
}
