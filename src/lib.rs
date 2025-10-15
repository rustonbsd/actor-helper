//! Minimal, self-contained actor runtime used in this crate.
//!
//! This module provides a tiny, opinionated actor pattern built on top of
//! `tokio`, with:
//!
//! - A per-actor mailbox (`mpsc::Receiver<Action<A>>`) of boxed async actions.
//! - A `Handle<A>` you can clone and send across threads to schedule work on the
//!   actor's single-threaded mutable state.
//! - Ergonomic macros (`act!`, `act_ok!`) to write actor actions inline.
//! - Panic capturing and error propagation to callers via `anyhow::Result`.
//!
//! The pattern encourages a public API type ("Object") that holds a
//! `Handle<ObjectActor>` and a private type ("ObjectActor") that owns the
//! mutable state and the mailbox `Receiver`. The actor implements `Actor` for a
//! simple `run` loop and is spawned with `tokio::spawn`.
//!
//! Example: a simple counter
//!
//! ```rust,ignore
//! use anyhow::{anyhow, Result};
//! use iroh_lan::actor::{Actor, Handle, Action};
//! use tokio::sync::mpsc;
//!
//! // Public API type (exposed from your module)
//! pub struct Counter {
//!     api: Handle<CounterActor>,
//! }
//!
//! impl Counter {
//!     pub fn new() -> Self {
//!         // 1) Create channel and api handle
//!         let (api, rx) = Handle::channel(128);
//!
//!         // 2) Create the actor with private state and the mailbox
//!         let actor = CounterActor { value: 0, rx };
//!
//!         // 3) Spawn the run loop
//!         tokio::spawn(async move {
//!             let mut actor = actor;
//!             let _ = actor.run().await;
//!         });
//!
//!         Self { api }
//!     }
//!
//!     // Mutating API method
//!     pub async fn inc(&self, by: i32) -> Result<()> {
//!         self.api
//!             // act_ok! wraps the returned value into Ok(..)
//!             .call(act_ok!(actor => async move {
//!                 actor.value += by;
//!             }))
//!             .await
//!     }
//!
//!     // Query method returning a value
//!     pub async fn get(&self) -> Result<i32> {
//!         self.api
//!             .call(act_ok!(actor => actor.value))
//!             .await
//!     }
//!
//!     // An example that can fail
//!     pub async fn set_non_negative(&self, v: i32) -> Result<()> {
//!         self.api
//!             .call(act!(actor => async move {
//!                 if v < 0 {
//!                     Err(anyhow!("negative value"))
//!                 } else {
//!                     actor.value = v;
//!                     Ok(())
//!                 }
//!             }))
//!             .await
//!     }
//! }
//!
//! // Private actor with its state and mailbox
//! struct CounterActor {
//!     value: i32,
//!     rx: mpsc::Receiver<Action<CounterActor>>,
//! }
//!
//! impl Actor for CounterActor {
//!     async fn run(&mut self) -> Result<()> {
//!         async move {
//!             loop {
//!                 tokio::select! {
//!                     Some(action) = self.rx.recv() => {
//!                         action(self).await;
//!                     }
//!                     Some(your_bytes) = your_reader.recv() => {
//!                         // do other background work if needed
//!                     }
//!                     // ... other background work ...
//!                 }
//!             }
//!             Ok(())
//!         }
//!     }
//! }
//!
//! # async fn _example_usage() -> Result<()> {
//! #   let c = Counter::new();
//! #   c.inc(5).await?;
//! #   assert_eq!(c.get().await?, 5);
//! #   Ok(())
//! # }
//! ```
//!
//! Notes
//! - `Handle::call` captures the call site location and wraps any panic from the
//!   actor action into an `anyhow::Error` delivered to the caller.
//! - Actions must complete reasonably promptly; the actor processes actions
//!   sequentially and a long-running action blocks the mailbox.
//! - Do not hold references across `.await` inside actions; prefer moving values
//!   or cloning as needed.
use std::{boxed, future::Future, io, pin::Pin};

use futures_util::FutureExt;

pub type Sender<T> = flume::Sender<T>;
pub type Receiver<T> = flume::Receiver<T>;

pub use futures_executor::block_on;

/// The unboxed future type used for actor actions.
///
/// This is a convenience type alias for a `Send` future that returns `T`.
/// Prefer using [`ActorFut`] which wraps this in a pinned `Box`.
pub type PreBoxActorFut<'a, T> = dyn Future<Output = T> + Send + 'a;

/// The boxed future type returned by actor actions.
///
/// Most APIs take or return `ActorFut<'a, T>` as the standard boxed future
/// used to execute actor code on the actor thread.
pub type ActorFut<'a, T> = Pin<boxed::Box<PreBoxActorFut<'a, T>>>;

/// An action scheduled onto an actor.
///
/// This is a boxed closure that receives a mutable reference to the actor
/// instance and returns a boxed future to execute. The future typically returns
/// `anyhow::Result<T>`, but `T` is generic here to support helper adapters.
pub type Action<A> = Box<dyn for<'a> FnOnce(&'a mut A) -> ActorFut<'a, ()> + Send + 'static>;

/// Convert a future yielding `anyhow::Result<T>` into the standard boxed
/// [`ActorFut`].
///
/// Use this when your actor code already returns `Result<T, anyhow::Error>`.
///
/// See also: [`into_actor_fut_ok`], [`into_actor_fut_unit_ok`].
pub fn into_actor_fut_res<'a, Fut, T>(fut: Fut) -> ActorFut<'a, io::Result<T>>
where
    Fut: Future<Output = io::Result<T>> + Send + 'a,
    T: Send + 'a,
{
    Box::pin(fut)
}

/// Convert a future yielding `T` into a boxed future yielding `anyhow::Result<T>`.
///
/// This wraps the output in `Ok(T)` for convenience.
pub fn into_actor_fut_ok<'a, Fut, T>(fut: Fut) -> ActorFut<'a, io::Result<T>>
where
    Fut: Future<Output = T> + Send + 'a,
    T: Send + 'a,
{
    Box::pin(async move { Ok::<T, io::Error>(fut.await) })
}

/// Convert a unit future (`Future<Output = ()>`) into a boxed future yielding
/// `anyhow::Result<()>`.
///
/// Convenient when an action does not return any value.
pub fn into_actor_fut_unit_ok<'a, Fut>(fut: Fut) -> ActorFut<'a, io::Result<()>>
where
    Fut: Future<Output = ()> + Send + 'a,
{
    Box::pin(async move {
        fut.await;
        Ok::<(), io::Error>(())
    })
}

/// Write an actor action that returns `anyhow::Result<T>`.
///
/// This macro helps create the closure expected by [`Handle::call`] when your
/// action returns `Result<T, anyhow::Error>`. Use `act_ok!` if your action
/// returns a plain `T`.
///
/// Examples
/// ```rust,ignore
/// Self.api.call(act!(actor => actor.do_something())).await
/// // or with a block
/// Self.api.call(act!(actor => async move { actor.do_something().await })).await
/// ```
#[macro_export]
macro_rules! act {
    // takes single expression that yields Result<T, anyhow::Error>
    ($actor:ident => $expr:expr) => {{ move |$actor| $crate::into_actor_fut_res(($expr)) }};

    // takes a block that yields Result<T, anyhow::Error> and can use ?
    ($actor:ident => $body:block) => {{ move |$actor| $crate::into_actor_fut_res($body) }};
}
/// Write an actor action that returns a plain `T` (wrapped as `Ok(T)`).
///
/// This macro is like [`act!`] but for actions that do not naturally return
/// an `anyhow::Result`. The output is automatically wrapped into `Ok(..)`.
///
/// Examples
/// ```rust,ignore
/// Self.api.call(act_ok!(actor => actor.get_value())).await
/// // or with a block
/// Self.api.call(act_ok!(actor => {
///   let v = actor.get_value();
///   v
/// })).await
/// ```
#[macro_export]
macro_rules! act_ok {
    // takes single expression that yields T
    ($actor:ident => $expr:expr) => {{ move |$actor| $crate::into_actor_fut_ok(($expr)) }};

    // takes a block that yields T (no = u) map to Ok(T)
    ($actor:ident => $body:block) => {{ move |$actor| $crate::into_actor_fut_ok($body) }};
}

/// A minimal trait implemented by concrete actor types to run their mailbox.
///
/// `async fn run()` is intendet to be alive as long as the actor is alive,
/// processing actions from the mailbox sequentially.
///
/// *Note:* This is also the place to do any continuous background work *your* actor
/// needs to perform.
///
/// The returned future should poll the mailbox and execute enqueued actions
/// until the channel closes.
#[cfg(any(feature = "tokio", feature = "async-std"))]
pub trait Actor: Send + 'static {
    fn run(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}

pub trait ActorSync: Send + 'static {
    fn run_blocking(&mut self) -> io::Result<()>;
}

pub fn spawn_actor_blocking<A>(actor: A) -> std::thread::JoinHandle<io::Result<()>>
where
    A: ActorSync,
{
    std::thread::spawn(move || {
        let mut actor = actor;
        actor.run_blocking()
    })
}

#[cfg(feature = "tokio")]
pub fn spawn_actor<A>(actor: A) -> tokio::task::JoinHandle<io::Result<()>>
where
    A: Actor,
{
    tokio::task::spawn(async move {
        let mut actor = actor;
        actor.run().await
    })
}

#[cfg(feature = "async-std")]
pub fn spawn_actor<A>(actor: A) -> async_std::task::JoinHandle<io::Result<()>>
where
    A: Actor,
{
    async_std::task::spawn(async move {
        let mut actor = actor;
        actor.run().await
    })
}

#[derive(Debug)]
/// A clonable handle to schedule actions onto an actor of type `A`.
///
/// Use [`Handle::channel`] to create a `(Handle<A>, mpsc::Receiver<Action<A>>)`
/// pair, then build your actor with the receiver and spawn its run loop. The
/// handle may be cloned and used concurrently from many tasks/threads.
pub struct Handle<A> {
    tx: Sender<Action<A>>,
}

impl<A> Clone for Handle<A> {
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
    /// Create a new actor handle and mailbox receiver with the given capacity.
    ///
    /// Returns the `(Handle<A>, mpsc::Receiver<Action<A>>)` pair. Pass the
    /// receiver to your actor and spawn its `run` loop like this:
    /// ```rust,ignore
    /// let (handle, rx) = Handle::<MyActor>::channel(128);
    /// let actor = MyActor { /* ... */ rx };
    /// tokio::spawn(async move { let _ = actor.run().await; });
    /// ```
    pub fn channel() -> (Self, Receiver<Action<A>>) {
        let (tx, rx) = flume::unbounded();
        (Self { tx }, rx)
    }

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
                    let res = std::panic::AssertUnwindSafe(async move { f(actor).await })
                        .catch_unwind()
                        .await
                        .map_err(|p| {
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

                    let _ = rtx.send(res);
                })
            }))
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("actor stopped (call send at {}:{})", loc.file(), loc.line()),
                )
            })?;
        Ok((rrx, loc))
    }

    /// Schedule an action to run on the actor and await its result.
    ///
    /// - `f` is a closure that receives `&mut A` and returns a boxed future
    ///   yielding `anyhow::Result<R>`. Use the [`act!`] and [`act_ok!`] macros
    ///   to write these concisely.
    /// - If the actor panics while processing the action, the panic is caught
    ///   and returned as an `anyhow::Error` with the call site location.
    /// - If the actor task has stopped, an error is returned.
    pub fn call_blocking<R, F>(&self, f: F) -> io::Result<R>
    where
        F: for<'a> FnOnce(&'a mut A) -> ActorFut<'a, io::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        let (rrx, loc) = self.base_call(f)?;

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

    /// Schedule an action to run on the actor and await its result.
    ///
    /// - `f` is a closure that receives `&mut A` and returns a boxed future
    ///   yielding `anyhow::Result<R>`. Use the [`act!`] and [`act_ok!`] macros
    ///   to write these concisely.
    /// - If the actor panics while processing the action, the panic is caught
    ///   and returned as an `anyhow::Error` with the call site location.
    /// - If the actor task has stopped, an error is returned.
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
