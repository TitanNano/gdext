use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use crate::builtin::{Callable, Signal, Variant};
use crate::classes::object::ConnectFlags;
use crate::godot_error;
use crate::meta::FromGodot;
use crate::obj::EngineEnum;

pub fn godot_task(future: impl Future<Output = ()> + 'static) {
    let waker: Waker = ASYNC_RUNTIME.with_borrow_mut(move |rt| {
        let task_index = rt.add_task(Box::pin(future));
        Arc::new(GodotWaker::new(task_index)).into()
    });

    waker.wake();
}

thread_local! { static ASYNC_RUNTIME: RefCell<AsyncRuntime> = RefCell::new(AsyncRuntime::new()); }

struct AsyncRuntime {
    tasks: Vec<Option<Pin<Box<dyn Future<Output = ()>>>>>,
}

impl AsyncRuntime {
    fn new() -> Self {
        Self {
            tasks: Vec::with_capacity(10),
        }
    }

    fn add_task<F: Future<Output = ()> + 'static>(&mut self, future: F) -> usize {
        let slot = self
            .tasks
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none());

        let boxed = Box::pin(future);

        match slot {
            Some((index, slot)) => {
                *slot = Some(boxed);
                index
            }
            None => {
                self.tasks.push(Some(boxed));
                self.tasks.len() - 1
            }
        }
    }

    fn get_task(&mut self, index: usize) -> Option<Pin<&mut (dyn Future<Output = ()> + 'static)>> {
        let slot = self.tasks.get_mut(index);

        slot.and_then(|inner| inner.as_mut())
            .map(|fut| fut.as_mut())
    }

    fn clear_task(&mut self, index: usize) {
        if index >= self.tasks.len() {
            return;
        }

        self.tasks[0] = None;
    }
}

struct GodotWaker {
    runtime_index: usize,
}

impl GodotWaker {
    fn new(index: usize) -> Self {
        Self {
            runtime_index: index,
        }
    }
}

impl Wake for GodotWaker {
    fn wake(self: std::sync::Arc<Self>) {
        let waker: Waker = self.clone().into();
        let mut ctx = Context::from_waker(&waker);

        ASYNC_RUNTIME.with_borrow_mut(|rt| {
            let Some(future) = rt.get_task(self.runtime_index) else {
                godot_error!("Future no longer exists! This is a bug!");
                return;
            };

            // this does currently not support nested tasks.
            let result = future.poll(&mut ctx);
            match result {
                Poll::Pending => (),
                Poll::Ready(()) => rt.clear_task(self.runtime_index),
            }
        });
    }
}

pub struct SignalFuture<R: FromSignalArgs> {
    state: Arc<Mutex<(Option<R>, Option<Waker>)>>,
}

impl<R: FromSignalArgs> SignalFuture<R> {
    fn new(signal: Signal) -> Self {
        let state = Arc::new(Mutex::new((None, Option::<Waker>::None)));
        let callback_state = state.clone();

        // the callable currently requires that the return value is Sync + Send
        signal.connect(
            Callable::from_fn("async_task", move |args: &[&Variant]| {
                let mut lock = callback_state.lock().unwrap();
                let waker = lock.1.take();

                lock.0.replace(R::from_args(args));
                drop(lock);

                if let Some(waker) = waker {
                    waker.wake();
                }

                Ok(Variant::nil())
            }),
            ConnectFlags::ONE_SHOT.ord() as i64,
        );

        Self { state }
    }
}

impl<R: FromSignalArgs> Future for SignalFuture<R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut lock = self.state.lock().unwrap();

        if let Some(result) = lock.0.take() {
            return Poll::Ready(result);
        }

        lock.1.replace(cx.waker().clone());

        Poll::Pending
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

// Signal should implement IntoFuture for convenience. Keeping ToSignalFuture around might still be desirable, though. It allows to reuse i
// the same signal instance multiple times.
pub trait ToSignalFuture<R: FromSignalArgs> {
    fn to_future(&self) -> SignalFuture<R>;
}

impl<R: FromSignalArgs> ToSignalFuture<R> for Signal {
    fn to_future(&self) -> SignalFuture<R> {
        SignalFuture::new(self.clone())
    }
}
