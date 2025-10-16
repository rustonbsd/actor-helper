//! Lightweight, runtime-agnostic actor helper library.
//!
//! This crate provides a minimal actor pattern that works with multiple async
//! runtimes (`tokio`, `async-std`) as well as synchronous/blocking code. It uses
//! [`flume`] channels for cross-thread communication.
//!
//! # Features
//!
//! - **Runtime agnostic**: Works with `tokio`, `async-std`, or no async runtime
//! - **Simple API**: Public types hold a `Handle<A>`, private actors own state
//! - **Type-safe**: Actions are strongly typed closures with futures
//! - **Panic safety**: Panics in actors are caught and returned as errors
//! - **Location tracking**: Error messages include call site location
//! - **Ergonomic macros**: `act!` and `act_ok!` for inline action creation
//!
//! # Architecture
//!
//! The recommended pattern:
//! 1. **Public API type** holds a `Handle<ActorType>` (can be cloned and shared)
//! 2. **Private actor type** owns mutable state and a `Receiver<Action<Self>>`
//! 3. **Actor trait** implementation runs a loop processing actions
//! 4. **Actions** are closures that receive `&mut Actor` and return futures
//!
//! # Examples
//!
//! ## Async with tokio
//!
//! ```rust,ignore
//! use std::io;
//! use actor_helper::{Actor, Handle, Receiver, act_ok, spawn_actor};
//!
//! // Public API
//! pub struct Counter {
//!     handle: Handle<CounterActor>,
//! }
//!
//! impl Counter {
//!     pub fn new() -> Self {
//!         let (handle, rx) = Handle::channel();
//!         spawn_actor(CounterActor { value: 0, rx });
//!         Self { handle }
//!     }
//!
//!     pub async fn increment(&self, by: i32) -> io::Result<()> {
//!         self.handle.call(act_ok!(actor => async move {
//!             actor.value += by;
//!         })).await
//!     }
//!
//!     pub async fn get(&self) -> io::Result<i32> {
//!         self.handle.call(act_ok!(actor => async move {
//!             actor.value
//!         })).await
//!     }
//! }
//!
//! // Private actor
//! struct CounterActor {
//!     value: i32,
//!     rx: Receiver<actor_helper::Action<CounterActor>>,
//! }
//!
//! impl Actor for CounterActor {
//!     async fn run(&mut self) -> io::Result<()> {
//!         loop {
//!             tokio::select! {
//!                 Ok(action) = self.rx.recv_async() => {
//!                     action(self).await;
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! ## Synchronous/Blocking
//!
//! ```rust,ignore
//! use std::io;
//! use actor_helper::{ActorSync, Handle, Receiver, act_ok, spawn_actor_blocking, block_on};
//!
//! struct CounterActor {
//!     value: i32,
//!     rx: Receiver<actor_helper::Action<CounterActor>>,
//! }
//!
//! impl ActorSync for CounterActor {
//!     fn run_blocking(&mut self) -> io::Result<()> {
//!         loop {
//!             if let Ok(action) = self.rx.recv() {
//!                 block_on(action(self));
//!             }
//!         }
//!     }
//! }
//!
//! pub struct Counter {
//!     handle: Handle<CounterActor>,
//! }
//!
//! impl Counter {
//!     pub fn new() -> Self {
//!         let (handle, rx) = Handle::channel();
//!         spawn_actor_blocking(CounterActor { value: 0, rx });
//!         Self { handle }
//!     }
//!
//!     pub fn increment(&self, by: i32) -> io::Result<()> {
//!         self.handle.call_blocking(act_ok!(actor => async move {
//!             actor.value += by;
//!         }))
//!     }
//! }
//! ```
//!
//! # Important Notes
//!
//! - Actions are processed **sequentially** - long-running actions block the mailbox
//! - Panics in actions are caught and returned as `io::Error` with location info
//! - Channels are **unbounded** to avoid deadlocks
//! - Do not hold references across `.await` in actions; move or clone instead
//! - The `call` method requires a feature flag (`tokio` or `async-std`)
//! - The `call_blocking` method works without any features
use std::{boxed, future::Future, io, pin::Pin};

use futures_util::FutureExt;

/// Type alias for flume's unbounded sender.
///
/// Used internally for sending actions to actors. Flume channels are chosen
/// for their performance and cross-runtime compatibility.
pub type Sender<T> = flume::Sender<T>;

/// Type alias for flume's unbounded receiver.
///
/// Actors own a `Receiver<Action<Self>>` to receive actions from their handle.
/// Use `recv()` for blocking reception or `recv_async()` for async reception.
pub type Receiver<T> = flume::Receiver<T>;

/// Re-export of `futures_executor::block_on` for convenience.
///
/// Use this in `ActorSync::run_blocking` to execute the async action futures
/// in a blocking context.
pub use futures_executor::block_on;

/// The unboxed future type used for actor actions (before boxing).
///
/// This is a `Send` future that returns `T`. In practice, you'll almost always
/// work with [`ActorFut`] which is this type already pinned and boxed.
pub type PreBoxActorFut<'a, T> = dyn Future<Output = T> + Send + 'a;

/// A pinned, boxed future returned by actor action helpers.
///
/// This is the standard future type used throughout the actor system. Helper
/// functions like `into_actor_fut_ok` and `into_actor_fut_res` return this type,
/// and the macros `act!` and `act_ok!` use these helpers internally.
pub type ActorFut<'a, T> = Pin<boxed::Box<PreBoxActorFut<'a, T>>>;

/// An action that can be scheduled onto an actor.
///
/// Actions are boxed closures that:
/// 1. Take a mutable reference to the actor (`&mut A`)
/// 2. Return a boxed future that yields `()` when executed
///
/// The actual return value of the action is communicated through a oneshot
/// channel created by `Handle::call` or `Handle::call_blocking`. The action
/// itself always returns `()` to the actor's run loop.
///
/// Use the `act!` and `act_ok!` macros to create actions conveniently.
pub type Action<A> = Box<dyn for<'a> FnOnce(&'a mut A) -> ActorFut<'a, ()> + Send + 'static>;

/// Convert a future yielding `io::Result<T>` into a boxed [`ActorFut`].
///
/// Use this helper when your action code already returns `io::Result<T>`.
/// The `act!` macro uses this helper internally.
///
/// # Example
/// ```rust,ignore
/// let action = move |actor: &mut MyActor| {
///     into_actor_fut_res(async move {
///         // Your code that returns io::Result<i32>
///         Ok(42)
///     })
/// };
/// ```
///
/// See also: [`into_actor_fut_ok`], [`into_actor_fut_unit_ok`].
pub fn into_actor_fut_res<'a, Fut, T>(fut: Fut) -> ActorFut<'a, io::Result<T>>
where
    Fut: Future<Output = io::Result<T>> + Send + 'a,
    T: Send + 'a,
{
    Box::pin(fut)
}

/// Convert a future yielding `T` into a boxed future yielding `io::Result<T>`.
///
/// This wraps the output in `Ok(T)` for convenience. Use this when your action
/// code cannot fail and you want it automatically wrapped in `Ok`.
/// The `act_ok!` macro uses this helper internally.
///
/// # Example
/// ```rust,ignore
/// let action = move |actor: &mut MyActor| {
///     into_actor_fut_ok(async move {
///         // Your code that returns i32 directly
///         actor.value + 1
///     })
/// };
/// ```
pub fn into_actor_fut_ok<'a, Fut, T>(fut: Fut) -> ActorFut<'a, io::Result<T>>
where
    Fut: Future<Output = T> + Send + 'a,
    T: Send + 'a,
{
    Box::pin(async move { Ok::<T, io::Error>(fut.await) })
}

/// Convert a unit future into a boxed future yielding `io::Result<()>`.
///
/// Convenient when an action performs work but doesn't return a meaningful value.
/// The future's `()` output is wrapped in `Ok(())`.
///
/// # Example
/// ```rust,ignore
/// let action = move |actor: &mut MyActor| {
///     into_actor_fut_unit_ok(async move {
///         actor.do_something();
///         // implicit ()
///     })
/// };
/// ```
pub fn into_actor_fut_unit_ok<'a, Fut>(fut: Fut) -> ActorFut<'a, io::Result<()>>
where
    Fut: Future<Output = ()> + Send + 'a,
{
    Box::pin(async move {
        fut.await;
        Ok::<(), io::Error>(())
    })
}

/// Create an actor action that returns `io::Result<T>`.
///
/// This macro helps create actions for use with `Handle::call` or
/// `Handle::call_blocking`. Use this when your action code can fail and
/// already returns `io::Result<T>`.
///
/// For actions that cannot fail, use [`act_ok!`] instead.
///
/// # Syntax
/// ```rust,ignore
/// // Single expression returning io::Result<T>
/// handle.call(act!(actor => some_method_returning_result(actor))).await?;
///
/// // Async block returning io::Result<T>
/// handle.call(act!(actor => async move {
///     if actor.value < 0 {
///         Err(io::Error::new(io::ErrorKind::Other, "negative value"))
///     } else {
///         Ok(actor.value)
///     }
/// })).await?;
/// ```
#[macro_export]
macro_rules! act {
    // Single expression that yields io::Result<T>
    ($actor:ident => $expr:expr) => {{ move |$actor| $crate::into_actor_fut_res(($expr)) }};

    // Block that yields io::Result<T> and can use the ? operator
    ($actor:ident => $body:block) => {{ move |$actor| $crate::into_actor_fut_res($body) }};
}

/// Create an actor action that returns a plain `T` (auto-wrapped as `Ok(T)`).
///
/// This macro is like [`act!`] but for actions that cannot fail. The return
/// value is automatically wrapped in `Ok(...)` to produce `io::Result<T>`.
///
/// # Syntax
/// ```rust,ignore
/// // Single expression returning T
/// handle.call(act_ok!(actor => actor.value)).await?;
///
/// // Async block returning T
/// handle.call(act_ok!(actor => async move {
///     actor.value += 1;
///     actor.value
/// })).await?;
/// ```
#[macro_export]
macro_rules! act_ok {
    // Single expression that yields T (not a Result)
    ($actor:ident => $expr:expr) => {{ move |$actor| $crate::into_actor_fut_ok(($expr)) }};

    // Block that yields T (not a Result), wrapped to produce io::Result<T>
    ($actor:ident => $body:block) => {{ move |$actor| $crate::into_actor_fut_ok($body) }};
}

/// Trait for async actors that process actions in a run loop.
///
/// Implement this trait for your private actor types when using async runtimes
/// (`tokio` or `async-std` features). The `run` method should loop forever,
/// receiving actions from the mailbox and executing them.
///
/// # Requirements
/// - The actor must be `Send + 'static` to be spawned on the runtime
/// - Use `recv_async()` on the receiver in async context
/// - Process actions sequentially to maintain actor semantics
///
/// # Example
/// ```rust,ignore
/// impl Actor for MyActor {
///     async fn run(&mut self) -> io::Result<()> {
///         loop {
///             tokio::select! {
///                 Ok(action) = self.rx.recv_async() => {
///                     action(self).await;
///                 }
///                 // Can include other branches for background work
///             }
///         }
///     }
/// }
/// ```
///
/// See also: [`ActorSync`] for synchronous/blocking actors.
#[cfg(any(feature = "tokio", feature = "async-std"))]
pub trait Actor: Send + 'static {
    fn run(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}

/// Trait for synchronous actors that process actions in a blocking run loop.
///
/// Implement this trait for your private actor types when not using async
/// runtimes. The `run_blocking` method should loop forever, receiving actions
/// from the mailbox and executing them with `block_on`.
///
/// # Requirements
/// - The actor must be `Send + 'static` to be spawned on a thread
/// - Use `recv()` on the receiver for blocking reception
/// - Use `block_on` to execute the action futures
///
/// # Example
/// ```rust,ignore
/// impl ActorSync for MyActor {
///     fn run_blocking(&mut self) -> io::Result<()> {
///         loop {
///             if let Ok(action) = self.rx.recv() {
///                 block_on(action(self));
///             }
///         }
///     }
/// }
/// ```
///
/// See also: [`Actor`] for async actors.
pub trait ActorSync: Send + 'static {
    fn run_blocking(&mut self) -> io::Result<()>;
}

/// Spawn a synchronous actor on a new OS thread.
///
/// Creates a new thread with `std::thread::spawn` and runs the actor's
/// `run_blocking` method. Returns a join handle that can be used to wait
/// for the actor to complete or retrieve errors.
///
/// # Example
/// ```rust,ignore
/// let (handle, rx) = Handle::channel();
/// let actor = MyActor { state: 0, rx };
/// let join_handle = spawn_actor_blocking(actor);
/// ```
pub fn spawn_actor_blocking<A>(actor: A) -> std::thread::JoinHandle<io::Result<()>>
where
    A: ActorSync,
{
    std::thread::spawn(move || {
        let mut actor = actor;
        actor.run_blocking()
    })
}

/// Spawn an async actor on the tokio runtime.
///
/// Creates a new tokio task and runs the actor's `run` method. Returns a
/// join handle that can be used to await the actor's completion or retrieve errors.
///
/// Only available with the `tokio` feature.
///
/// # Example
/// ```rust,ignore
/// let (handle, rx) = Handle::channel();
/// let actor = MyActor { state: 0, rx };
/// let join_handle = spawn_actor(actor);
/// ```
#[cfg(all(feature = "tokio", not(feature = "async-std")))]
pub fn spawn_actor<A>(actor: A) -> tokio::task::JoinHandle<io::Result<()>>
where
    A: Actor,
{
    tokio::task::spawn(async move {
        let mut actor = actor;
        actor.run().await
    })
}

/// Spawn an async actor on the async-std runtime.
///
/// Creates a new async-std task and runs the actor's `run` method. Returns a
/// join handle that can be used to await the actor's completion or retrieve errors.
///
/// Only available with the `async-std` feature.
///
/// # Example
/// ```rust,ignore
/// let (handle, rx) = Handle::channel();
/// let actor = MyActor { state: 0, rx };
/// let join_handle = spawn_actor(actor);
/// ```
#[cfg(all(feature = "async-std", not(feature = "tokio")))]
pub fn spawn_actor<A>(actor: A) -> async_std::task::JoinHandle<io::Result<()>>
where
    A: Actor,
{
    async_std::task::spawn(async move {
        let mut actor = actor;
        actor.run().await
    })
}

/// A cloneable handle to schedule actions on an actor of type `A`.
///
/// The handle is the primary interface for interacting with an actor. It can
/// be cloned freely and shared across threads/tasks. Each call to `call` or
/// `call_blocking` schedules an action to run on the actor.
///
/// # Thread Safety
/// `Handle<A>` is `Send + Sync` and can be cloned to share across threads.
/// This is safe because the actor itself runs sequentially, processing actions
/// one at a time.
///
/// # Creation
/// Use [`Handle::channel`] to create a handle and its corresponding receiver:
/// ```rust,ignore
/// let (handle, rx) = Handle::<MyActor>::channel();
/// ```
#[derive(Debug)]
pub struct Handle<A> {
    tx: Sender<Action<A>>,
}

impl<A> Clone for Handle<A> {
    /// Clone the handle to create another reference to the same actor.
    ///
    /// All clones send actions to the same actor instance.
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<A> Handle<A>
where
    A: Send + 'static,
{
    /// Create a new actor handle and unbounded mailbox receiver.
    ///
    /// Returns a `(Handle<A>, Receiver<Action<A>>)` pair. Give the receiver
    /// to your actor and spawn its run loop. Keep the handle to send actions.
    ///
    /// The channel is unbounded to avoid potential deadlocks when the actor
    /// needs to make calls to itself or when multiple handles send concurrently.
    ///
    /// # Example
    /// ```rust,ignore
    /// let (handle, rx) = Handle::<MyActor>::channel();
    /// let actor = MyActor { state: 0, rx };
    /// spawn_actor(actor);
    /// // Now use `handle` to interact with the actor
    /// ```
    pub fn channel() -> (Self, Receiver<Action<A>>) {
        let (tx, rx) = flume::unbounded();
        (Self { tx }, rx)
    }

    /// Internal helper for both `call` and `call_blocking`.
    ///
    /// This method:
    /// 1. Creates a oneshot channel for the result
    /// 2. Wraps the action with panic catching and result forwarding
    /// 3. Sends the wrapped action to the actor
    /// 4. Returns the result receiver and call location for error messages
    ///
    /// The wrapped action executes `f(actor)`, catches any panics, and sends
    /// the result (or panic error) back through the oneshot channel.
    fn base_call<R, F>(
        &self,
        f: F,
    ) -> io::Result<(
        Receiver<io::Result<R>>,
        &'static std::panic::Location<'static>,
    )>
    where
        F: for<'a> FnOnce(&'a mut A) -> ActorFut<'a, io::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        let (rtx, rrx) = flume::unbounded();
        let loc = std::panic::Location::caller();

        self.tx
            .send(Box::new(move |actor: &mut A| {
                Box::pin(async move {
                    // Execute the action and catch any panics
                    let res = std::panic::AssertUnwindSafe(async move { f(actor).await })
                        .catch_unwind()
                        .await
                        .map_err(|p| {
                            // Convert panic payload to error message
                            let msg = if let Some(s) = p.downcast_ref::<&str>() {
                                (*s).to_string()
                            } else if let Some(s) = p.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic".to_string()
                            };
                            io::Error::new(
                                io::ErrorKind::Other,
                                format!(
                                    "panic in actor call at {}:{}: {}",
                                    loc.file(),
                                    loc.line(),
                                    msg
                                ),
                            )
                        })
                        .and_then(|r| r);

                    // Send result back to caller (ignore send errors - caller may have dropped)
                    let _ = rtx.send(res);
                })
            }))
            .map_err(|_| {
                // Actor's receiver was dropped, meaning the actor stopped
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("actor stopped (call send at {}:{})", loc.file(), loc.line()),
                )
            })?;
        Ok((rrx, loc))
    }

    /// Schedule an action on the actor and block until it completes.
    ///
    /// This is the synchronous version of `call` that works without async
    /// runtimes. It blocks the current thread until the action completes.
    ///
    /// # Parameters
    /// - `f`: A closure that receives `&mut A` and returns a boxed future
    ///   yielding `io::Result<R>`. Use `act!` or `act_ok!` to create these.
    ///
    /// # Returns
    /// - `Ok(R)` if the action completed successfully
    /// - `Err(io::Error)` if the action failed, panicked, or the actor stopped
    ///
    /// # Error Cases
    /// - Action returns an error: The error is propagated
    /// - Action panics: Converted to `io::Error` with call location
    /// - Actor stopped: `io::Error` indicating the actor is no longer running
    ///
    /// # Blocking Behavior
    /// Uses a polling loop with 10ms timeout intervals and `yield_now()` to
    /// avoid busy-waiting while still providing reasonably low latency.
    ///
    /// # Example
    /// ```rust,ignore
    /// let result = handle.call_blocking(act_ok!(actor => async move {
    ///     actor.value += 1;
    ///     actor.value
    /// }))?;
    /// ```
    pub fn call_blocking<R, F>(&self, f: F) -> io::Result<R>
    where
        F: for<'a> FnOnce(&'a mut A) -> ActorFut<'a, io::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        let (rrx, loc) = self.base_call(f)?;

        // Poll for result with timeouts to avoid blocking forever
        loop {
            match rrx.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(result) => return result,
                Err(flume::RecvTimeoutError::Timeout) => {
                    std::thread::yield_now();
                    continue;
                }
                Err(flume::RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("actor stopped at {}:{}", loc.file(), loc.line()),
                    ));
                }
            }
        }
    }

    /// Schedule an action on the actor and await its result.
    ///
    /// This is the async version of `call_blocking`. It awaits the action's
    /// completion without blocking the current thread.
    ///
    /// Only available with the `tokio` or `async-std` feature enabled.
    ///
    /// # Parameters
    /// - `f`: A closure that receives `&mut A` and returns a boxed future
    ///   yielding `io::Result<R>`. Use `act!` or `act_ok!` to create these.
    ///
    /// # Returns
    /// - `Ok(R)` if the action completed successfully
    /// - `Err(io::Error)` if the action failed, panicked, or the actor stopped
    ///
    /// # Error Cases
    /// - Action returns an error: The error is propagated
    /// - Action panics: Converted to `io::Error` with call location
    /// - Actor stopped: `io::Error` indicating the actor is no longer running
    ///
    /// # Example
    /// ```rust,ignore
    /// let result = handle.call(act_ok!(actor => async move {
    ///     actor.value += 1;
    ///     actor.value
    /// })).await?;
    /// ```
    #[cfg(any(feature = "tokio", feature = "async-std"))]
    pub async fn call<R, F>(&self, f: F) -> io::Result<R>
    where
        F: for<'a> FnOnce(&'a mut A) -> ActorFut<'a, io::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        let (rrx, loc) = self.base_call(f)?;
        rrx.recv_async().await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("actor stopped (call recv at {}:{})", loc.file(), loc.line()),
            )
        })?
    }
}
