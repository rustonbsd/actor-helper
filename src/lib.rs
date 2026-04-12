//! Lightweight, runtime-agnostic actor pattern with dynamic error types.
//!
//! Works with `tokio`, `async-std`, or blocking threads. Uses [`flume`] channels.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use std::io;
//! use actor_helper::{Handle, act_ok};
//!
//! // Public API
//! pub struct Counter {
//!     handle: Handle<CounterActor, io::Error>,
//! }
//!
//! impl Counter {
//!     pub fn new() -> Self {
//!         Self { handle: Handle::spawn(CounterActor { value: 0 }).0 }
//!     }
//!
//!     pub async fn increment(&self, by: i32) -> io::Result<()> {
//!         self.handle.call(act_ok!(actor => async move {
//!             actor.value += by;
//!         })).await
//!     }
//!
//!     pub async fn get(&self) -> io::Result<i32> {
//!         self.handle.call(act_ok!(actor => async move { actor.value })).await
//!     }
//!
//!     pub async fn is_running(&self) -> bool {
//!         self.handle.state() == ActorState::Running
//!     }
//! }
//!
//! // Private actor
//! struct CounterActor {
//!     value: i32,
//! }
//! ```
//!
//! # Error Types
//!
//! Use any error type implementing [`ActorError`]:
//! - `io::Error` (default)
//! - `anyhow::Error` (with `anyhow` feature)
//! - `String`
//! - `Box<dyn Error>`
//!
//! # Blocking/Sync
//!
//! ```rust,ignore
//! // Use spawn_blocking and call_blocking instead of spawn and call
//! let (handle, _) = Handle::<MyActor, io::Error>::spawn_blocking(MyActor { value: 0 });
//! handle.call_blocking(act_ok!(actor => async move { actor.value }))?;
//! ```
//!
//! # Custom Loops
//!
//! Use `spawn_with` / `spawn_blocking_with` for custom receive loops (e.g. `tokio::select!`):
//!
//! ```rust,ignore
//! use actor_helper::{Handle, Receiver, Action};
//!
//! let (handle, _) = Handle::<MyActor, io::Error>::spawn_with(
//!     MyActor { value: 0 },
//!     |mut actor, rx| async move {
//!         loop {
//!             tokio::select! {
//!                 Ok(action) = rx.recv_async() => action(&mut actor).await,
//!                 _ = some_background_task() => { /* ... */ },
//!                 else => break Ok(()),
//!             }
//!         }
//!     },
//! );
//! ```
//!
//! # Notes
//!
//! - Actions run sequentially, long tasks block the mailbox
//! - Panics are caught and converted to errors with location info
//! - `call` requires `tokio` or `async-std` feature
//! - `call_blocking` has no feature requirements
use std::{
    any::Any,
    boxed,
    collections::HashMap,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

#[allow(unused_imports)]
use futures_util::{
    FutureExt,
    future::{self, Either},
    pin_mut,
};

/// Flume unbounded sender.
pub type Sender<T> = flume::Sender<T>;

/// Flume unbounded receiver. Actors receive actions via `Receiver<Action<Self>>`.
///
/// Use `recv()` for blocking or `recv_async()` for async.
pub type Receiver<T> = flume::Receiver<T>;

/// Execute async futures in blocking context.
pub use futures_executor::block_on;

/// Convert panic/actor-stop messages into your error type.
///
/// Implemented for `io::Error`, `anyhow::Error`, `String`, and `Box<dyn Error>`.
///
/// # Example
/// ```rust,ignore
/// impl ActorError for MyError {
///     fn from_actor_message(msg: String) -> Self {
///         MyError::ActorPanic(msg)
///     }
/// }
/// ```
pub trait ActorError: Sized + Send + 'static {
    fn from_actor_message(msg: String) -> Self;
}

// Implementations for common types
impl ActorError for io::Error {
    fn from_actor_message(msg: String) -> Self {
        io::Error::other(msg)
    }
}

/// Represents the current state of an actor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ActorState {
    #[default]
    Running,
    Stopped,
}

#[cfg(feature = "anyhow")]
impl ActorError for anyhow::Error {
    fn from_actor_message(msg: String) -> Self {
        anyhow::anyhow!(msg)
    }
}

impl ActorError for String {
    fn from_actor_message(msg: String) -> Self {
        msg
    }
}

impl ActorError for Box<dyn std::error::Error + Send + Sync> {
    fn from_actor_message(msg: String) -> Self {
        Box::new(io::Error::other(msg))
    }
}

/// Unboxed future type for actor actions.
pub type PreBoxActorFut<'a, T> = dyn Future<Output = T> + Send + 'a;

/// Pinned, boxed future used by action helpers and macros.
pub type ActorFut<'a, T> = Pin<boxed::Box<PreBoxActorFut<'a, T>>>;

/// Action sent to an actor: `FnOnce(&mut A) -> Future<()>`.
///
/// Created via `act!` or `act_ok!` macros. Return values flow through oneshot channels.
pub type Action<A> = Box<dyn for<'a> FnOnce(&'a mut A) -> ActorFut<'a, ()> + Send + 'static>;

/// Internal result type used by `Handle::base_call`.
type BaseCallResult<R, E> = Result<
    (
        Receiver<Result<R, E>>,
        Receiver<()>,
        u64,
        &'static std::panic::Location<'static>,
    ),
    E,
>;

type PendingCancelMap = Arc<Mutex<HashMap<u64, Sender<()>>>>;

fn fail_pending_calls(pending: &PendingCancelMap) {
    if let Ok(mut pending) = pending.lock() {
        for (_, cancel_tx) in pending.drain() {
            let _ = cancel_tx.send(());
        }
    }
}

/// Box a future yielding `Result<T, E>`. Used by `act!` macro.
#[doc(hidden)]
pub fn into_actor_fut_res<'a, Fut, T, E>(fut: Fut) -> ActorFut<'a, Result<T, E>>
where
    Fut: Future<Output = Result<T, E>> + Send + 'a,
    T: Send + 'a,
{
    Box::pin(fut)
}

/// Box a future yielding `T`, wrap as `Ok(T)`. Used by `act_ok!` macro.
#[doc(hidden)]
pub fn into_actor_fut_ok<'a, Fut, T, E>(fut: Fut) -> ActorFut<'a, Result<T, E>>
where
    Fut: Future<Output = T> + Send + 'a,
    T: Send + 'a,
    E: ActorError,
{
    Box::pin(async move { Ok(fut.await) })
}

/// Create action returning `Result<T, E>`.
///
/// # Example
/// ```rust,ignore
/// handle.call(act!(actor => async move {
///     if actor.value < 0 {
///         Err(io::Error::new(io::ErrorKind::Other, "negative"))
///     } else {
///         Ok(actor.value)
///     }
/// })).await?
/// ```
#[macro_export]
macro_rules! act {
    ($actor:ident => $expr:expr) => {{ move |$actor| $crate::into_actor_fut_res(($expr)) }};
    ($actor:ident => $body:block) => {{ move |$actor| $crate::into_actor_fut_res($body) }};
}

/// Create action returning `T`, auto-wrapped as `Ok(T)`.
///
/// # Example
/// ```rust,ignore
/// handle.call(act_ok!(actor => async move {
///     actor.value += 1;
///     actor.value
/// })).await?
/// ```
#[macro_export]
macro_rules! act_ok {
    ($actor:ident => $expr:expr) => {{ move |$actor| $crate::into_actor_fut_ok(($expr)) }};
    ($actor:ident => $body:block) => {{ move |$actor| $crate::into_actor_fut_ok($body) }};
}



fn panic_payload_message(panic_payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = panic_payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn actor_loop_panic<E: ActorError>(panic_payload: Box<dyn Any + Send>) -> E {
    E::from_actor_message(format!(
        "panic in actor loop: {}",
        panic_payload_message(panic_payload)
    ))
}


/// Cloneable handle to send actions to actor `A` with error type `E`.
///
/// Thread-safe. Actions run sequentially on the actor.
#[derive(Debug)]
pub struct Handle<A, E>
where
    A: Send + 'static,
    E: ActorError,
{
    tx: Arc<Mutex<Option<Sender<Action<A>>>>>,
    state: Arc<RwLock<ActorState>>,
    pending: PendingCancelMap,
    next_call_id: Arc<AtomicU64>,
    stopped_rx: Receiver<()>,
    _phantom: std::marker::PhantomData<E>,
}

impl<A, E> Clone for Handle<A, E>
where
    A: Send + 'static,
    E: ActorError,
{
    fn clone(&self) -> Self {
        Self {
            tx: Arc::clone(&self.tx),
            state: Arc::clone(&self.state),
            pending: Arc::clone(&self.pending),
            next_call_id: Arc::clone(&self.next_call_id),
            stopped_rx: self.stopped_rx.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<A, E> PartialEq for Handle<A, E>
where
    A: Send + 'static,
    E: ActorError,
{
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl<A, E> Eq for Handle<A, E>
where
    A: Send + 'static,
    E: ActorError,
{
}

impl<A, E> Handle<A, E>
where
    A: Send + 'static,
    E: ActorError,
{
    /// Read the current lifecycle state of the actor.
    pub fn state(&self) -> ActorState {
        self.state.read().expect("poisned lock").clone()
    }

    /// Spawn an async actor with the default message loop (requires tokio or async-std).
    ///
    /// The library runs: receive action → execute → repeat, until the channel disconnects.
    /// Use [`spawn_with`](Self::spawn_with) for custom receive loops.
    #[cfg(all(feature = "tokio", not(feature = "async-std")))]
    pub fn spawn(actor: A) -> (Self, tokio::task::JoinHandle<Result<(), E>>)
    {
        let (tx, rx) = flume::unbounded::<Action<A>>();
        let state = Arc::new(RwLock::new(ActorState::default()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let next_call_id = Arc::new(AtomicU64::new(0));
        let (stopped_tx, stopped_rx) = flume::bounded::<()>(1);

        let join_handle = {
            let state = Arc::clone(&state);
            let pending = Arc::clone(&pending);
            tokio::task::spawn(async move {
                let _stopped_signal = stopped_tx;
                let mut actor = actor;

                let res = std::panic::AssertUnwindSafe(async {
                    while let Ok(action) = rx.recv_async().await {
                        action(&mut actor).await;
                    }
                    Ok::<(), E>(())
                })
                .catch_unwind()
                .await;

                if let Ok(mut st) = state.write() {
                    *st = ActorState::Stopped;
                }
                fail_pending_calls(&pending);
                match res {
                    Ok(result) => result,
                    Err(panic_payload) => Err(actor_loop_panic(panic_payload)),
                }
            })
        };

        (
            Self {
                tx: Arc::new(Mutex::new(Some(tx))),
                state,
                pending,
                next_call_id,
                stopped_rx,
                _phantom: std::marker::PhantomData,
            },
            join_handle,
        )
    }

    /// Spawn an async actor with a custom run loop (requires tokio).
    ///
    /// The closure receives ownership of the actor and the action receiver.
    /// Use `action(&mut actor).await` to execute received actions.
    #[cfg(all(feature = "tokio", not(feature = "async-std")))]
    pub fn spawn_with<F, Fut>(actor: A, run: F) -> (Self, tokio::task::JoinHandle<Result<(), E>>)
    where
        F: FnOnce(A, Receiver<Action<A>>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send,
    {
        let (tx, rx) = flume::unbounded();
        let state = Arc::new(RwLock::new(ActorState::default()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let next_call_id = Arc::new(AtomicU64::new(0));
        let (stopped_tx, stopped_rx) = flume::bounded::<()>(1);

        let join_handle = {
            let state = Arc::clone(&state);
            let pending = Arc::clone(&pending);
            tokio::task::spawn(async move {
                let _stopped_signal = stopped_tx;

                let res = std::panic::AssertUnwindSafe(run(actor, rx))
                    .catch_unwind()
                    .await;

                if let Ok(mut st) = state.write() {
                    *st = ActorState::Stopped;
                }
                fail_pending_calls(&pending);
                match res {
                    Ok(result) => result,
                    Err(panic_payload) => Err(actor_loop_panic(panic_payload)),
                }
            })
        };

        (
            Self {
                tx: Arc::new(Mutex::new(Some(tx))),
                state,
                pending,
                next_call_id,
                stopped_rx,
                _phantom: std::marker::PhantomData,
            },
            join_handle,
        )
    }

    /// Spawn an async actor with the default message loop (requires async-std).
    ///
    /// The library runs: receive action → execute → repeat, until the channel disconnects.
    /// Use [`spawn_with`](Self::spawn_with) for custom receive loops.
    #[cfg(all(feature = "async-std", not(feature = "tokio")))]
    pub fn spawn(actor: A) -> (Self, async_std::task::JoinHandle<Result<(), E>>)
    {
        let (tx, rx) = flume::unbounded::<Action<A>>();
        let state = Arc::new(RwLock::new(ActorState::default()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let next_call_id = Arc::new(AtomicU64::new(0));
        let (stopped_tx, stopped_rx) = flume::bounded::<()>(1);

        let join_handle = {
            let state = Arc::clone(&state);
            let pending = Arc::clone(&pending);
            async_std::task::spawn(async move {
                let _stopped_signal = stopped_tx;
                let mut actor = actor;

                let res = std::panic::AssertUnwindSafe(async {
                    while let Ok(action) = rx.recv_async().await {
                        action(&mut actor).await;
                    }
                    Ok::<(), E>(())
                })
                .catch_unwind()
                .await;

                if let Ok(mut st) = state.write() {
                    *st = ActorState::Stopped;
                }
                fail_pending_calls(&pending);
                match res {
                    Ok(result) => result,
                    Err(panic_payload) => Err(actor_loop_panic(panic_payload)),
                }
            })
        };

        (
            Self {
                tx: Arc::new(Mutex::new(Some(tx))),
                state,
                pending,
                next_call_id,
                stopped_rx,
                _phantom: std::marker::PhantomData,
            },
            join_handle,
        )
    }

    /// Spawn an async actor with a custom run loop (requires async-std).
    ///
    /// The closure receives ownership of the actor and the action receiver.
    /// Use `action(&mut actor).await` to execute received actions.
    #[cfg(all(feature = "async-std", not(feature = "tokio")))]
    pub fn spawn_with<F, Fut>(actor: A, run: F) -> (Self, async_std::task::JoinHandle<Result<(), E>>)
    where
        F: FnOnce(A, Receiver<Action<A>>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send,
    {
        let (tx, rx) = flume::unbounded();
        let state = Arc::new(RwLock::new(ActorState::default()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let next_call_id = Arc::new(AtomicU64::new(0));
        let (stopped_tx, stopped_rx) = flume::bounded::<()>(1);

        let join_handle = {
            let state = Arc::clone(&state);
            let pending = Arc::clone(&pending);
            async_std::task::spawn(async move {
                let _stopped_signal = stopped_tx;

                let res = std::panic::AssertUnwindSafe(run(actor, rx))
                    .catch_unwind()
                    .await;

                if let Ok(mut st) = state.write() {
                    *st = ActorState::Stopped;
                }
                fail_pending_calls(&pending);
                match res {
                    Ok(result) => result,
                    Err(panic_payload) => Err(actor_loop_panic(panic_payload)),
                }
            })
        };

        (
            Self {
                tx: Arc::new(Mutex::new(Some(tx))),
                state,
                pending,
                next_call_id,
                stopped_rx,
                _phantom: std::marker::PhantomData,
            },
            join_handle,
        )
    }

    /// Spawn a blocking actor with the default message loop on a new OS thread.
    ///
    /// The library runs: receive action → execute → repeat, until the channel disconnects.
    /// Use [`spawn_blocking_with`](Self::spawn_blocking_with) for custom receive loops.
    pub fn spawn_blocking(actor: A) -> (Self, std::thread::JoinHandle<Result<(), E>>)
    {
        let (tx, rx) = flume::unbounded::<Action<A>>();
        let state = Arc::new(RwLock::new(ActorState::default()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let next_call_id = Arc::new(AtomicU64::new(0));
        let (stopped_tx, stopped_rx) = flume::bounded::<()>(1);

        let join_handle = {
            let state = Arc::clone(&state);
            let pending = Arc::clone(&pending);
            std::thread::spawn(move || {
                let _stopped_signal = stopped_tx;
                let mut actor = actor;

                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    while let Ok(action) = rx.recv() {
                        block_on(action(&mut actor));
                    }
                    Ok::<(), E>(())
                }));

                if let Ok(mut st) = state.write() {
                    *st = ActorState::Stopped;
                }
                fail_pending_calls(&pending);
                match res {
                    Ok(result) => result,
                    Err(panic_payload) => Err(actor_loop_panic(panic_payload)),
                }
            })
        };

        (
            Self {
                tx: Arc::new(Mutex::new(Some(tx))),
                state,
                pending,
                next_call_id,
                stopped_rx,
                _phantom: std::marker::PhantomData,
            },
            join_handle,
        )
    }

    /// Spawn a blocking actor with a custom run loop on a new OS thread.
    ///
    /// The closure receives ownership of the actor and the action receiver.
    /// Use `block_on(action(&mut actor))` to execute received actions.
    pub fn spawn_blocking_with<F>(actor: A, run: F) -> (Self, std::thread::JoinHandle<Result<(), E>>)
    where
        F: FnOnce(A, Receiver<Action<A>>) -> Result<(), E> + Send + 'static,
    {
        let (tx, rx) = flume::unbounded();
        let state = Arc::new(RwLock::new(ActorState::default()));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let next_call_id = Arc::new(AtomicU64::new(0));
        let (stopped_tx, stopped_rx) = flume::bounded::<()>(1);

        let join_handle = {
            let state = Arc::clone(&state);
            let pending = Arc::clone(&pending);
            std::thread::spawn(move || {
                let _stopped_signal = stopped_tx;

                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(actor, rx)
                }));

                if let Ok(mut st) = state.write() {
                    *st = ActorState::Stopped;
                }
                fail_pending_calls(&pending);
                match res {
                    Ok(result) => result,
                    Err(panic_payload) => Err(actor_loop_panic(panic_payload)),
                }
            })
        };

        (
            Self {
                tx: Arc::new(Mutex::new(Some(tx))),
                state,
                pending,
                next_call_id,
                stopped_rx,
                _phantom: std::marker::PhantomData,
            },
            join_handle,
        )
    }

    /// Internal: wraps action with panic catching and result forwarding.
    fn base_call<R, F>(&self, f: F) -> BaseCallResult<R, E>
    where
        F: for<'a> FnOnce(&'a mut A) -> ActorFut<'a, Result<R, E>> + Send + 'static,
        R: Send + 'static,
    {
        if self.state() != ActorState::Running {
            return Err(E::from_actor_message(
                "actor stopped (call attempted while actor state is not running)".to_string(),
            ));
        }

        let (rtx, rrx) = flume::unbounded();
        let (cancel_tx, cancel_rx) = flume::bounded::<()>(1);
        let loc = std::panic::Location::caller();
        let call_id = self.next_call_id.fetch_add(1, Ordering::Relaxed);
        self.pending
            .lock()
            .expect("poisoned lock")
            .insert(call_id, cancel_tx);

        let action: Action<A> = Box::new(move |actor: &mut A| {
            Box::pin(async move {
                // Execute the action and catch any panics
                let panic_result = std::panic::AssertUnwindSafe(async move { f(actor).await })
                    .catch_unwind()
                    .await;

                let res = match panic_result {
                    Ok(action_result) => action_result,
                    Err(panic_payload) => {
                        // Convert panic payload to error message
                        let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "unknown panic".to_string()
                        };
                        Err(E::from_actor_message(format!(
                            "panic in actor call at {}:{}: {}",
                            loc.file(),
                            loc.line(),
                            msg
                        )))
                    }
                };

                // Send result back to caller (ignore send errors - caller may have dropped)
                let _ = rtx.send(res);
            })
        });

        let sent = {
            let tx_guard = self.tx.lock().expect("poisoned lock");
            tx_guard
                .as_ref()
                .map_or(false, |tx| tx.send(action).is_ok())
        };

        if !sent {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&call_id);
            }
            return Err(E::from_actor_message(format!(
                "actor stopped (call send at {}:{})",
                loc.file(),
                loc.line()
            )));
        }

        Ok((rrx, cancel_rx, call_id, loc))
    }

    /// Send action, block until complete. Works without async runtime.
    ///
    /// # Example
    /// ```rust,ignore
    /// handle.call_blocking(act_ok!(actor => async move {
    ///     actor.value += 1;
    ///     actor.value
    /// }))?
    /// ```
    pub fn call_blocking<R, F>(&self, f: F) -> Result<R, E>
    where
        F: for<'a> FnOnce(&'a mut A) -> ActorFut<'a, Result<R, E>> + Send + 'static,
        R: Send + 'static,
    {
        enum BlockingWaitResult<T, E> {
            Result(Result<Result<T, E>, flume::RecvError>),
            Canceled(Result<(), flume::RecvError>),
        }

        let (rrx, cancel_rx, call_id, loc) = self.base_call(f)?;
        let out = match flume::Selector::new()
            .recv(&rrx, BlockingWaitResult::Result)
            .recv(&cancel_rx, BlockingWaitResult::Canceled)
            .wait()
        {
            BlockingWaitResult::Result(msg) => msg.map_err(|_| {
                E::from_actor_message(format!(
                    "actor stopped (call recv at {}:{})",
                    loc.file(),
                    loc.line()
                ))
            })?,
            BlockingWaitResult::Canceled(Ok(())) => Err(E::from_actor_message(format!(
                "actor stopped (call canceled at {}:{})",
                loc.file(),
                loc.line()
            ))),
            BlockingWaitResult::Canceled(Err(_)) => Err(E::from_actor_message(format!(
                "actor stopped (call recv at {}:{})",
                loc.file(),
                loc.line()
            ))),
        };

        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&call_id);
        }

        out
    }

    /// Send action, await result. Requires `tokio` or `async-std` feature.
    ///
    /// # Example
    /// ```rust,ignore
    /// handle.call(act_ok!(actor => async move {
    ///     actor.value += 1;
    ///     actor.value
    /// })).await?
    /// ```
    #[cfg(any(feature = "tokio", feature = "async-std"))]
    pub async fn call<R, F>(&self, f: F) -> Result<R, E>
    where
        F: for<'a> FnOnce(&'a mut A) -> ActorFut<'a, Result<R, E>> + Send + 'static,
        R: Send + 'static,
    {
        let (rrx, cancel_rx, call_id, loc) = self.base_call(f)?;

        let recv_fut = rrx.recv_async();
        let cancel_fut = cancel_rx.recv_async();
        pin_mut!(recv_fut, cancel_fut);

        let out = match future::select(recv_fut, cancel_fut).await {
            Either::Left((msg, _)) => msg.map_err(|_| {
                E::from_actor_message(format!(
                    "actor stopped (call recv at {}:{})",
                    loc.file(),
                    loc.line()
                ))
            })?,
            Either::Right((Ok(_), _)) => Err(E::from_actor_message(format!(
                "actor stopped (call canceled at {}:{})",
                loc.file(),
                loc.line()
            ))),
            Either::Right((Err(_), _)) => Err(E::from_actor_message(format!(
                "actor stopped (call recv at {}:{})",
                loc.file(),
                loc.line()
            ))),
        };

        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&call_id);
        }

        out
    }

    /// Disconnect the action channel, causing the actor's `recv()` to return `Err`.
    ///
    /// The actor loop should exit naturally when `recv()` returns `Err`.
    /// Already-queued messages will be drained before the channel reports disconnection.
    /// Does nothing if already shut down.
    pub fn shutdown(&self) {
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
    }

    /// Block until the actor has stopped. Returns immediately if already stopped.
    pub fn wait_stopped_blocking(&self) {
        if self.state() == ActorState::Stopped {
            return;
        }
        let _ = self.stopped_rx.recv();
    }

    /// Wait asynchronously until the actor has stopped. Returns immediately if already stopped.
    #[cfg(any(feature = "tokio", feature = "async-std"))]
    pub async fn wait_stopped(&self) {
        if self.state() == ActorState::Stopped {
            return;
        }
        let _ = self.stopped_rx.recv_async().await;
    }
}