#[cfg(test)]
#[cfg(feature = "tokio")]
mod tokio_tests {
    
use std::{io, sync::Arc};

use actor_helper::{Action, Actor, Handle, Receiver, spawn_actor};
use actor_helper::{act, act_ok};
use tokio;

struct TestActor {
    value: i32,
    rx: Receiver<Action<TestActor>>,
}

impl Actor for TestActor {
    async fn run(&mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => {
                    action(self).await;
                }
            }
        }
    }
}

struct TestApi {
    handle: Handle<TestActor>,
}

impl TestApi {
    fn new() -> Self {
        let (handle, rx) = Handle::channel();
        let actor = TestActor { value: 0, rx };

        let _join_handle = spawn_actor(actor);

        Self { handle }
    }

    async fn set_value(&self, value: i32) -> io::Result<()> {
        self.handle.call(act_ok!(actor => async move {
            actor.value = value;
        })).await
    }

    async fn get_value(&self) -> io::Result<i32> {
        self.handle.call(act_ok!(actor => async move {
                actor.value
        })).await
    }

    async fn increment(&self, by: i32) -> io::Result<()> {
        self.handle.call(act_ok!(actor => async move {
            actor.value += by;
        })).await
    }

    async fn set_positive(&self, value: i32) -> io::Result<()> {
        self.handle.call(act!(actor => async move {
            if value <= 0 {
                Err(io::Error::new(io::ErrorKind::Other, "Value must be positive"))
            } else {
                actor.value = value;
                Ok(())
            }
        })).await
    }

    async fn multiply(&self, factor: i32) -> io::Result<i32> {
        self.handle.call(act_ok!(actor => async move {
            actor.value *= factor;
            actor.value
        })).await
    }
}

#[tokio::test]
async fn test_basic_operations() {
    let api = TestApi::new();

    assert_eq!(api.get_value().await.unwrap(), 0);

    api.set_value(42).await.unwrap();
    assert_eq!(api.get_value().await.unwrap(), 42);

    api.increment(8).await.unwrap();
    assert_eq!(api.get_value().await.unwrap(), 50);
}

#[tokio::test]
async fn test_error_handling() {
    let api = TestApi::new();

    let result = api.set_positive(-5).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("positive"));

    api.set_positive(10).await.unwrap();
    assert_eq!(api.get_value().await.unwrap(), 10);
}

#[tokio::test]
async fn test_return_values() {
    let api = TestApi::new();

    api.set_value(7).await.unwrap();
    let result = api.multiply(3).await.unwrap();
    assert_eq!(result, 21);
    assert_eq!(api.get_value().await.unwrap(), 21);
}

#[tokio::test]
async fn test_concurrent_access() {
    let api = Arc::new(TestApi::new());
    let mut handles = vec![];

    for i in 0..10 {
        let api_clone = api.clone();
        let handle = api_clone.increment(i).await.unwrap();
        handles.push(handle);
    }

    let final_value = api.get_value().await.unwrap();
    assert_eq!(final_value, 45);
}

#[tokio::test]
async fn test_sequential_operations() {
    let api = TestApi::new();

    api.set_value(1).await.unwrap();
    for _ in 0..5 {
        let current = api.get_value().await.unwrap();
        api.set_value(current * 2).await.unwrap();
    }

    assert_eq!(api.get_value().await.unwrap(), 32);
}

#[tokio::test]
async fn test_clone_handle() {
    let api1 = TestApi::new();
    let api2 = TestApi {
        handle: api1.handle.clone(),
    };

    api1.set_value(100).await.unwrap();
    assert_eq!(api2.get_value().await.unwrap(), 100);

    api2.increment(50).await.unwrap();
    assert_eq!(api1.get_value().await.unwrap(), 150);
}

struct CounterActor {
    count: i32,
    rx: Receiver<Action<CounterActor>>,
}

impl Actor for CounterActor {
    async fn run(&mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                Ok(action) = self.rx.recv_async() => {
                    action(self).await;
                }
            }
        }
    }
}

#[tokio::test]
async fn test_shared_state() {
    let (handle, rx) = Handle::channel();
    let actor = CounterActor { count: 0, rx };

    let _join_handle = spawn_actor(actor);

    handle
        .call(act_ok!(actor => async move {
            actor.count += 1;
        }))
        .await
        .unwrap();

    assert_eq!(
        handle
            .call(act_ok!(actor => async move { actor.count }))
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn test_async_action() {
    let api = TestApi::new();

    api.handle
        .call(act!(actor => async move {
            std::thread::sleep(std::time::Duration::from_millis(10));
            actor.value = 999;
            Ok(())
        }))
        .await
        .unwrap();

    assert_eq!(api.get_value().await.unwrap(), 999);
}

#[tokio::test]
async fn test_multiple_handles_same_actor() {
    let (handle1, rx) = Handle::channel();
    let handle2 = handle1.clone();
    let handle3 = handle1.clone();

    let actor = TestActor { value: 0, rx };
    let _join_handle = spawn_actor(actor);

    handle1
        .call(act_ok!(actor => async move { actor.value += 10; }))
        .await
        .unwrap();
    handle2
        .call(act_ok!(actor => async move { actor.value *= 2; }))
        .await
        .unwrap();
    let result = handle3
        .call(act_ok!(actor => async move { actor.value }))
        .await
        .unwrap();

    assert_eq!(result, 20);
}

}