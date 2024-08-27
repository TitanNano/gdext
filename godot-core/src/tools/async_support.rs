use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, ThreadId};

use crate::builtin::{Callable, Signal, Variant};
use crate::classes::object::ConnectFlags;
use crate::classes::Os;
use crate::meta::{FromGodot, ToGodot};
use crate::obj::EngineEnum;

pub fn godot_task(future: impl Future<Output = ()> + 'static) {
    let os = Os::singleton();

    // Spawning new tasks is only allowed on the main thread for now.
    // We can not accept Sync + Send futures since all object references (i.e. Gd<T>) are not thread-safe. So a future has to remain on the
    // same thread it was created on. Godots signals on the other hand can be emitted on any thread, so it can't be guaranteed on which thread
    // a future will be polled.
    // By limiting async tasks to the main thread we can redirect all signal callbacks back to the main thread via `call_deferred`.
    //
    // Once thread-safe futures are possible the restriction can be lifted.
    if os.get_thread_caller_id() != os.get_main_thread_id() {
        return;
    }

    let waker: Waker = ASYNC_RUNTIME.with_borrow_mut(move |rt| {
        let task_index = rt.add_task(Box::pin(future));
        Arc::new(GodotWaker::new(task_index, thread::current().id())).into()
    });

    waker.wake();
}

thread_local! { pub(crate) static ASYNC_RUNTIME: RefCell<AsyncRuntime> = RefCell::new(AsyncRuntime::new()); }

#[derive(Default)]
enum FutureSlot<T> {
    #[default]
    Empty,
    Pending(T),
    Polling,
}

impl<T> FutureSlot<T> {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    fn clear(&mut self) {
        *self = Self::Empty;
    }

    fn take(&mut self) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Pending(_) => std::mem::replace(self, Self::Polling),
            Self::Polling => Self::Polling,
        }
    }

    fn park(&mut self, value: T) {
        match self {
            Self::Empty => {
                panic!("Future slot is currently unoccupied, future can not be parked here!");
            }

            Self::Pending(_) => panic!("Future slot is already occupied by a different future!"),
            Self::Polling => {
                *self = Self::Pending(value);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct AsyncRuntime {
    tasks: Vec<FutureSlot<Pin<Box<dyn Future<Output = ()>>>>>,
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
            .find(|(_, slot)| slot.is_empty());

        let boxed = Box::pin(future);

        match slot {
            Some((index, slot)) => {
                *slot = FutureSlot::Pending(boxed);
                index
            }
            None => {
                self.tasks.push(FutureSlot::Pending(boxed));
                self.tasks.len() - 1
            }
        }
    }

    fn get_task(
        &mut self,
        index: usize,
    ) -> FutureSlot<Pin<Box<dyn Future<Output = ()> + 'static>>> {
        let slot = self.tasks.get_mut(index);

        slot.map(|inner| inner.take()).unwrap_or_default()
    }

    fn clear_task(&mut self, index: usize) {
        if index >= self.tasks.len() {
            return;
        }

        self.tasks[0].clear();
    }

    fn park_task(&mut self, index: usize, future: Pin<Box<dyn Future<Output = ()>>>) {
        self.tasks[index].park(future);
    }
}

struct GodotWaker {
    runtime_index: usize,
    thread_id: ThreadId,
}

impl GodotWaker {
    fn new(index: usize, thread_id: ThreadId) -> Self {
        Self {
            runtime_index: index,
            thread_id,
        }
    }
}

impl Wake for GodotWaker {
    fn wake(self: std::sync::Arc<Self>) {
        let callable = Callable::from_fn("GodotWaker::wake", move |_args| {
            let current_thread = thread::current().id();

            if self.thread_id != current_thread {
                panic!("trying to poll future on a different thread!\nCurrent Thread: {:?}, Future Thread: {:?}", current_thread, self.thread_id);
            }

            let waker: Waker = self.clone().into();
            let mut ctx = Context::from_waker(&waker);

            // take future out of the runtime.
            let mut future = ASYNC_RUNTIME.with_borrow_mut(|rt| {
                match rt.get_task(self.runtime_index) {
                    FutureSlot::Empty => {
                        panic!("Future no longer exists when waking it! This is a bug!");
                    },

                    FutureSlot::Polling => {
                        panic!("The same GodotWaker has been called recursively, this is not expected!");
                    }

                    FutureSlot::Pending(future) => future
                }
            });

            let result = future.as_mut().poll(&mut ctx);

            // update runtime.
            ASYNC_RUNTIME.with_borrow_mut(|rt| match result {
                Poll::Pending => rt.park_task(self.runtime_index, future),
                Poll::Ready(()) => rt.clear_task(self.runtime_index),
            });

            Ok(Variant::nil())
        });

        // shedule waker to poll the future on the end of the frame.
        callable.to_variant().call("call_deferred", &[]);
    }
}

pub struct SignalFuture<R: FromSignalArgs> {
    state: Arc<Mutex<(Option<R>, Option<Waker>)>>,
    callable: Callable,
    signal: Signal,
}

impl<R: FromSignalArgs> SignalFuture<R> {
    fn new(signal: Signal) -> Self {
        let state = Arc::new(Mutex::new((None, Option::<Waker>::None)));
        let callback_state = state.clone();

        // the callable currently requires that the return value is Sync + Send
        let callable = Callable::from_fn("async_task", move |args: &[&Variant]| {
            let mut lock = callback_state.lock().unwrap();
            let waker = lock.1.take();

            lock.0.replace(R::from_args(args));
            drop(lock);

            if let Some(waker) = waker {
                waker.wake();
            }

            Ok(Variant::nil())
        });

        signal.connect(callable.clone(), ConnectFlags::ONE_SHOT.ord() as i64);

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

        if let Some(result) = lock.0.take() {
            return Poll::Ready(result);
        }

        lock.1.replace(cx.waker().clone());

        Poll::Pending
    }
}

impl<R: FromSignalArgs> Drop for SignalFuture<R> {
    fn drop(&mut self) {
        if !self.callable.is_valid() {
            return;
        }

        if self.signal.object().is_none() {
            return;
        }

        if self.signal.is_connected(self.callable.clone()) {
            self.signal.disconnect(self.callable.clone());
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
