/*
 * Copyright (c) godot-rust; Bromeon and contributors.
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::builtin::{Callable, RustCallable, Variant};
use crate::classes::object::ConnectFlags;
use crate::meta::FromGodot;
use crate::obj::EngineEnum;

use super::Signal;

struct SignalFutureState<Output> {
    output: Option<Output>,
    waker: Option<Waker>,
}

// Not derived, otherwise an extra bound `Output: Default` is required.
impl<Output> Default for SignalFutureState<Output> {
    fn default() -> Self {
        Self {
            output: None,
            waker: None,
        }
    }
}

pub struct SignalFuture<R: FromSignalArgs> {
    state: Arc<Mutex<SignalFutureState<R>>>,
    callable: Callable,
    signal: Signal,
}

impl<R: FromSignalArgs> SignalFuture<R> {
    fn new(signal: Signal) -> Self {
        let state = Arc::new(Mutex::new(SignalFutureState::<R>::default()));
        let callback_state = state.clone();

        #[cfg(not(feature = "experimental-threads"))]
        let create_callable = Callable::from_local_fn;

        #[cfg(feature = "experimental-threads")]
        let create_callable = Callable::from_sync_fn;

        // The callable requires that the return value is Sync + Send.
        let callable = create_callable("SignalFuture::resolve", move |args: &[&Variant]| {
            let mut lock = callback_state.lock().unwrap();
            let waker = lock.waker.take();

            lock.output.replace(R::from_args(args));
            drop(lock);

            if let Some(waker) = waker {
                waker.wake();
            }

            Ok(Variant::nil())
        });

        signal.connect(&callable, ConnectFlags::ONE_SHOT.ord() as i64);

        Self {
            state,
            callable,
            signal,
        }
    }
}

impl<R: FromSignalArgs> Future for SignalFuture<R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut lock = self.state.lock().unwrap();

        if let Some(result) = lock.output.take() {
            return Poll::Ready(result);
        }

        lock.waker.replace(cx.waker().clone());

        Poll::Pending
    }
}

impl<R: FromSignalArgs> Drop for SignalFuture<R> {
    fn drop(&mut self) {
        // The callable might alredy be destroyed, this occurs during engine shutdown.
        if !self.callable.is_valid() {
            return;
        }

        // If the future gets dropped after the signal object was already freed, it will be None. This can occur when the object is freed
        // and later the async task is canceled.
        if self.signal.object().is_none() {
            return;
        }

        if self.signal.is_connected(&self.callable) {
            self.signal.disconnect(&self.callable);
        }
    }
}

// Only public for itest.
#[cfg_attr(feature = "trace", derive(Default))]
pub struct GuaranteedSignalFutureResolver<R> {
    state: Arc<Mutex<(GuaranteedSignalFutureState<R>, Option<Waker>)>>,
}

impl<R> Clone for GuaranteedSignalFutureResolver<R> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<R> GuaranteedSignalFutureResolver<R> {
    fn new(state: Arc<Mutex<(GuaranteedSignalFutureState<R>, Option<Waker>)>>) -> Self {
        Self { state }
    }
}

impl<R> std::hash::Hash for GuaranteedSignalFutureResolver<R> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_usize(Arc::as_ptr(&self.state) as usize);
    }
}

impl<R> PartialEq for GuaranteedSignalFutureResolver<R> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl<R: FromSignalArgs> RustCallable for GuaranteedSignalFutureResolver<R> {
    fn invoke(&mut self, args: &[&Variant]) -> Result<Variant, ()> {
        let mut lock = self.state.lock().unwrap();
        let (state, waker) = &mut *lock;
        let waker = waker.take();

        *state = GuaranteedSignalFutureState::Ready(R::from_args(args));
        drop(lock);

        if let Some(waker) = waker {
            waker.wake();
        }

        Ok(Variant::nil())
    }
}

impl<R> Display for GuaranteedSignalFutureResolver<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GuaranteedSignalFutureResolver::<{}>",
            std::any::type_name::<R>()
        )
    }
}

// this resolver will resolve the future when it's being dropped (i.e. the engine removes all connected signal callables). This is very unusual.
impl<R> Drop for GuaranteedSignalFutureResolver<R> {
    fn drop(&mut self) {
        let mut lock = self.state.lock().unwrap();
        let (state, waker) = &mut *lock;

        if !matches!(state, GuaranteedSignalFutureState::Pending) {
            return;
        }

        *state = GuaranteedSignalFutureState::Dead;

        if let Some(waker) = waker {
            waker.wake_by_ref();
        }
    }
}

#[derive(Default)]
enum GuaranteedSignalFutureState<T> {
    #[default]
    Pending,
    Ready(T),
    Dead,
    Dropped,
}

impl<T> GuaranteedSignalFutureState<T> {
    fn take(&mut self) -> Self {
        let new_value = match self {
            Self::Pending => Self::Pending,
            Self::Ready(_) | Self::Dead => Self::Dead,
            Self::Dropped => Self::Dropped,
        };

        std::mem::replace(self, new_value)
    }
}

/// The guaranteed signal future will always resolve, but might resolve to `None` if the owning object is freed
/// before the signal is emitted.
///
/// This is inconsistent with how awaiting signals in Godot work and how async works in rust. The behavior was requested as part of some
/// user feedback for the initial POC.
pub struct GuaranteedSignalFuture<R: FromSignalArgs> {
    state: Arc<Mutex<(GuaranteedSignalFutureState<R>, Option<Waker>)>>,
    callable: GuaranteedSignalFutureResolver<R>,
    signal: Signal,
}

impl<R: FromSignalArgs> GuaranteedSignalFuture<R> {
    fn new(signal: Signal) -> Self {
        let state = Arc::new(Mutex::new((
            GuaranteedSignalFutureState::Pending,
            Option::<Waker>::None,
        )));

        // The callable currently requires that the return value is Sync + Send.
        let callable = GuaranteedSignalFutureResolver::new(state.clone());

        signal.connect(
            &Callable::from_custom(callable.clone()),
            ConnectFlags::ONE_SHOT.ord() as i64,
        );

        Self {
            state,
            callable,
            signal,
        }
    }
}

impl<R: FromSignalArgs> Future for GuaranteedSignalFuture<R> {
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut lock = self.state.lock().unwrap();
        let (state, waker) = &mut *lock;

        waker.replace(cx.waker().clone());

        let value = state.take();

        match value {
            GuaranteedSignalFutureState::Pending => Poll::Pending,
            GuaranteedSignalFutureState::Dropped => unreachable!(),
            GuaranteedSignalFutureState::Dead => Poll::Ready(None),
            GuaranteedSignalFutureState::Ready(value) => Poll::Ready(Some(value)),
        }
    }
}

impl<R: FromSignalArgs> Drop for GuaranteedSignalFuture<R> {
    fn drop(&mut self) {
        // The callable might alredy be destroyed, this occurs during engine shutdown.
        if self.signal.object().is_none() {
            return;
        }

        let mut lock = self.state.lock().unwrap();
        let (state, _) = &mut *lock;

        *state = GuaranteedSignalFutureState::Dropped;

        let gd_callable = Callable::from_custom(self.callable.clone());

        if self.signal.is_connected(&gd_callable) {
            self.signal.disconnect(&gd_callable);
        }
    }
}

pub trait FromSignalArgs: Sync + Send + 'static {
    fn from_args(args: &[&Variant]) -> Self;
}

impl<R: FromGodot + Sync + Send + 'static> FromSignalArgs for R {
    fn from_args(args: &[&Variant]) -> Self {
        args.first()
            .map(|arg| (*arg).to_owned())
            .unwrap_or_default()
            .to()
    }
}

// more of these should be generated via macro to support more than two signal arguments
impl<R1: FromGodot + Sync + Send + 'static, R2: FromGodot + Sync + Send + 'static> FromSignalArgs
    for (R1, R2)
{
    fn from_args(args: &[&Variant]) -> Self {
        (args[0].to(), args[0].to())
    }
}

impl Signal {
    pub fn to_guaranteed_future<R: FromSignalArgs>(&self) -> GuaranteedSignalFuture<R> {
        GuaranteedSignalFuture::new(self.clone())
    }

    pub fn to_future<R: FromSignalArgs>(&self) -> SignalFuture<R> {
        SignalFuture::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::sys;
    use std::sync::Arc;

    use super::GuaranteedSignalFutureResolver;

    /// Test that the hash of a cloned future resolver is equal to its original version. With this equality in place, we can create new
    /// Callables that are equal to their original version but have separate reference counting.
    #[test]
    fn guaranteed_future_resolver_cloned_hash() {
        let resolver_a = GuaranteedSignalFutureResolver::<u8>::new(Arc::default());
        let resolver_b = resolver_a.clone();

        let hash_a = sys::hash_value(&resolver_a);
        let hash_b = sys::hash_value(&resolver_b);

        assert_eq!(hash_a, hash_b);
    }
}
