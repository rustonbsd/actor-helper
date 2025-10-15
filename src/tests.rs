#[cfg(test)]
mod tests {
    use std::{io, sync::Arc};

    use futures_executor::block_on;

    use crate::{Action, Actor, Handle};
    use std::sync::mpsc;

    use crate::{act, act_ok};

    struct TestActor {
        value: i32,
        rx: mpsc::Receiver<Action<TestActor>>,
    }

    impl Actor for TestActor {
        async fn run(&mut self) -> io::Result<()> {
            loop {
                if let Ok(action) = self.rx.recv() {
                    println!("Received an action");
                    action(self).await;
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
            std::thread::spawn(move || {
                let mut actor = actor;
                let _ = futures_executor::block_on(actor.run());
            });

            Self { handle }
        }

        fn set_value(&self, value: i32) -> io::Result<()> {
            self.handle
                .call(act_ok!(actor => async move {
                    actor.value = value;
                }))
        }

        fn get_value(&self) -> io::Result<i32> {
            self.handle
                .call(act_ok!(actor => async move {
                        actor.value
                }))
        }

        fn increment(&self, by: i32) -> io::Result<()> {
            self.handle
                .call(act_ok!(actor => async move {
                    actor.value += by;
                }))
        }

        fn set_positive(&self, value: i32) -> io::Result<()> {
            self.handle
                .call(act!(actor => async move {
                    if value <= 0 {
                        Err(io::Error::new(io::ErrorKind::Other, "Value must be positive"))
                    } else {
                        actor.value = value;
                        Ok(())
                    }
                }))
        }

        fn multiply(&self, factor: i32) -> io::Result<i32> {
            self.handle
                .call(act_ok!(actor => async move {
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
        let mut handles = vec![];

        for i in 0..10 {
            let api_clone = api.clone();
            let handle = api_clone.increment(i).unwrap();
            handles.push(handle);
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
        rx: mpsc::Receiver<Action<CounterActor>>,
    }

    impl Actor for CounterActor {
        async fn run(&mut self) -> io::Result<()> {
            while let Ok(action) = self.rx.recv() {
                action(self).await;
            }
            Ok(())
        }
    }

    #[test]
    fn test_shared_state() {
        let (handle, rx) = Handle::channel();
        let actor = CounterActor { count: 0, rx };

        std::thread::spawn( move ||  {
            let mut actor = actor;
            let _ = futures_executor::block_on(actor.run());
        });

        handle
            .call(act_ok!(actor => async move {
                actor.count += 1;
            }))
            .unwrap();

        assert_eq!(
            handle
                .call(act_ok!(actor => async move { actor.count }))
                .unwrap(),
            1
        );
    }

    #[test]
    fn test_async_action() {
        let api = TestApi::new();

        api.handle
            .call(act!(actor => async move {
                std::thread::sleep(std::time::Duration::from_millis(10));
                actor.value = 999;
                Ok(())
            }))
            .unwrap();

        assert_eq!(api.get_value().unwrap(), 999);
    }

    
    #[test]
    fn test_multiple_handles_same_actor() {
        let (handle1, rx) = Handle::channel();
        let handle2 = handle1.clone();
        let handle3 = handle1.clone();

        let actor = TestActor { value: 0, rx };
        std::thread::spawn(move || {
            let mut actor = actor;
            let _ = block_on(actor.run());
        });

        handle1
            .call(act_ok!(actor => async move { actor.value += 10; }))
            .unwrap();
        handle2
            .call(act_ok!(actor => async move { actor.value *= 2; }))
            .unwrap();
        let result = handle3
            .call(act_ok!(actor => async move { actor.value }))
            .unwrap();

        assert_eq!(result, 20);
    }
}
