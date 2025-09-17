#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{Action, Actor, Handle};
    use anyhow::{Result, anyhow};
    use tokio::sync::mpsc;

    use crate::{act, act_ok};

    struct TestActor {
        value: i32,
        rx: mpsc::Receiver<Action<TestActor>>,
    }

    impl Actor for TestActor {
        async fn run(&mut self) -> Result<()> {
            loop {
                tokio::select! {
                    Some(action) = self.rx.recv() => {
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
            let (handle, rx) = Handle::channel(128);
            let actor = TestActor { value: 0, rx };

            tokio::spawn(async move {
                let mut actor = actor;
                let _ = actor.run().await;
            });

            Self { handle }
        }

        async fn set_value(&self, value: i32) -> Result<()> {
            self.handle
                .call(act_ok!(actor => async move {
                    actor.value = value;
                }))
                .await
        }

        async fn get_value(&self) -> Result<i32> {
            self.handle
                .call(act_ok!(actor => async move {
                        actor.value
                }))
                .await
        }

        async fn increment(&self, by: i32) -> Result<()> {
            self.handle
                .call(act_ok!(actor => async move {
                    actor.value += by;
                }))
                .await
        }

        async fn set_positive(&self, value: i32) -> Result<()> {
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

        async fn multiply(&self, factor: i32) -> Result<i32> {
            self.handle
                .call(act_ok!(actor => async move {
                    actor.value *= factor;
                    actor.value
                }))
                .await
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
            let handle = tokio::spawn(async move {
                api_clone.increment(i).await.unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
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
        rx: mpsc::Receiver<Action<CounterActor>>,
    }

    impl Actor for CounterActor {
        async fn run(&mut self) -> Result<()> {
            while let Some(action) = self.rx.recv().await {
                action(self).await;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_shared_state() {
        let (handle, rx) = Handle::channel(128);
        let actor = CounterActor { count: 0, rx };

        tokio::spawn(async move {
            let mut actor = actor;
            let _ = actor.run().await;
        });

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
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                actor.value = 999;
                Ok(())
            }))
            .await
            .unwrap();

        assert_eq!(api.get_value().await.unwrap(), 999);
    }

    #[tokio::test]
    async fn test_multiple_handles_same_actor() {
        let (handle1, rx) = Handle::channel(128);
        let handle2 = handle1.clone();
        let handle3 = handle1.clone();

        let actor = TestActor { value: 0, rx };
        tokio::spawn(async move {
            let mut actor = actor;
            let _ = actor.run().await;
        });

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
