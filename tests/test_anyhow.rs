#[cfg(test)]
#[cfg(feature = "anyhow")]
mod anyhow_tests {
    use std::sync::Arc;

    use actor_helper::Handle;
    use actor_helper::{act, act_ok};

    struct TestActor {
        value: i32,
    }

    struct TestApi {
        handle: Handle<TestActor, anyhow::Error>,
    }

    impl TestApi {
        fn new() -> Self {
            Self {
                handle: Handle::spawn_blocking(TestActor { value: 0 }).0,
            }
        }

        fn set_value(&self, value: i32) -> anyhow::Result<()> {
            self.handle.call_blocking(act_ok!(actor => async move {
                actor.value = value;
            }))
        }

        fn get_value(&self) -> anyhow::Result<i32> {
            self.handle.call_blocking(act_ok!(actor => async move {
                    actor.value
            }))
        }

        fn increment(&self, by: i32) -> anyhow::Result<()> {
            self.handle.call_blocking(act_ok!(actor => async move {
                actor.value += by;
            }))
        }

        fn set_positive(&self, value: i32) -> anyhow::Result<()> {
            self.handle.call_blocking(act!(actor => async move {
                if value <= 0 {
                    Err(anyhow::anyhow!("Value must be positive"))
                } else {
                    actor.value = value;
                    Ok(())
                }
            }))
        }

        fn multiply(&self, factor: i32) -> anyhow::Result<i32> {
            self.handle.call_blocking(act_ok!(actor => async move {
                actor.value *= factor;
                actor.value
            }))
        }
    }

    #[test]
    fn test_basic_operations() {
        let api = TestApi::new();

        assert_eq!(api.get_value().unwrap(), 0);

        api.set_value(42).unwrap();
        assert_eq!(api.get_value().unwrap(), 42);

        api.increment(8).unwrap();
        assert_eq!(api.get_value().unwrap(), 50);
    }

    #[test]
    fn test_error_handling() {
        let api = TestApi::new();

        let result = api.set_positive(-5);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("positive"));

        api.set_positive(10).unwrap();
        assert_eq!(api.get_value().unwrap(), 10);
    }

    #[test]
    fn test_return_values() {
        let api = TestApi::new();

        api.set_value(7).unwrap();
        let result = api.multiply(3).unwrap();
        assert_eq!(result, 21);
        assert_eq!(api.get_value().unwrap(), 21);
    }

    #[test]
    fn test_concurrent_access() {
        let api = Arc::new(TestApi::new());
        for i in 0..10 {
            let api_clone = api.clone();
            api_clone.increment(i).unwrap();
        }

        let final_value = api.get_value().unwrap();
        assert_eq!(final_value, 45);
    }

    #[test]
    fn test_sequential_operations() {
        let api = TestApi::new();

        api.set_value(1).unwrap();
        for _ in 0..5 {
            let current = api.get_value().unwrap();
            api.set_value(current * 2).unwrap();
        }

        assert_eq!(api.get_value().unwrap(), 32);
    }

    #[test]
    fn test_clone_handle() {
        let api1 = TestApi::new();
        let api2 = TestApi {
            handle: api1.handle.clone(),
        };

        api1.set_value(100).unwrap();
        assert_eq!(api2.get_value().unwrap(), 100);

        api2.increment(50).unwrap();
        assert_eq!(api1.get_value().unwrap(), 150);
    }

    struct CounterActor {
        count: i32,
    }

    #[test]
    fn test_shared_state() {
        let (handle, _) =
            Handle::<CounterActor, anyhow::Error>::spawn_blocking(CounterActor { count: 0 });

        handle
            .call_blocking(act_ok!(actor => async move {
                actor.count += 1;
            }))
            .unwrap();

        assert_eq!(
            handle
                .call_blocking(act_ok!(actor => async move { actor.count }))
                .unwrap(),
            1
        );
    }

    #[test]
    fn test_async_action() {
        let api = TestApi::new();

        api.handle
            .call_blocking(act_ok!(actor => async move {
                std::thread::sleep(std::time::Duration::from_millis(10));
                actor.value = 999;
            }))
            .unwrap();

        assert_eq!(api.get_value().unwrap(), 999);
    }

    #[test]
    fn test_multiple_handles_same_actor() {
        let (handle1, _) =
            Handle::<TestActor, anyhow::Error>::spawn_blocking(TestActor { value: 0 });
        let handle2 = handle1.clone();
        let handle3 = handle1.clone();

        handle1
            .call_blocking(act_ok!(actor => async move { actor.value += 10; }))
            .unwrap();
        handle2
            .call_blocking(act_ok!(actor => async move { actor.value *= 2; }))
            .unwrap();
        let result = handle3
            .call_blocking(act_ok!(actor => async move { actor.value }))
            .unwrap();

        assert_eq!(result, 20);
    }

    struct PanicActor;

    #[test]
    fn test_actor_loop_panic_is_returned_as_error() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let (_handle, join_handle) =
            Handle::<PanicActor, anyhow::Error>::spawn_blocking_with(PanicActor, |_, _| {
                panic!("blocking actor panic");
            });
        let result = join_handle.join().unwrap();
        let error = result.unwrap_err().to_string();

        std::panic::set_hook(prev);
        assert!(error.contains("panic in actor loop"));
        assert!(error.contains("blocking actor panic"));
    }
}
