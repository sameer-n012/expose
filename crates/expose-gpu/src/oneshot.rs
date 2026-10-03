//! A minimal single-value channel that can be awaited.
//!
//! wgpu reports a finished buffer map through a callback. On native targets,
//! `Device::poll` runs the callback. In a browser, the event loop runs it
//! later. This channel turns the callback into a future that works in both
//! cases, without a thread and without an async runtime.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

struct Shared<T> {
    value: Option<T>,
    waker: Option<Waker>,
    sender_dropped: bool,
}

/// Locks the shared state.
///
/// A poisoned lock can only come from a panic in this module while the lock
/// is held. The code below does not panic while it holds the lock, and the
/// state stays consistent in all cases, so it is safe to continue.
fn lock<T>(shared: &Mutex<Shared<T>>) -> MutexGuard<'_, Shared<T>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The sending half. Dropping it without a send closes the channel.
pub(crate) struct Sender<T> {
    shared: Arc<Mutex<Shared<T>>>,
}

/// The receiving half. Resolves to `Some(value)`, or to `None` if the sender
/// was dropped without a send.
pub(crate) struct Receiver<T> {
    shared: Arc<Mutex<Shared<T>>>,
}

/// Creates a connected sender and receiver.
pub(crate) fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Mutex::new(Shared {
        value: None,
        waker: None,
        sender_dropped: false,
    }));
    (
        Sender {
            shared: Arc::clone(&shared),
        },
        Receiver { shared },
    )
}

impl<T> Sender<T> {
    /// Sends the value and wakes the receiver.
    pub(crate) fn send(self, value: T) {
        let waker = {
            let mut state = lock(&self.shared);
            state.value = Some(value);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        // `Drop` runs next. It sets `sender_dropped`, but the value is
        // already stored, so the receiver gets the value.
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let waker = {
            let mut state = lock(&self.shared);
            state.sender_dropped = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Future for Receiver<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = lock(&self.shared);
        if let Some(value) = state.value.take() {
            return Poll::Ready(Some(value));
        }
        if state.sender_dropped {
            return Poll::Ready(None);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::channel;

    #[test]
    fn value_arrives() {
        let (sender, receiver) = channel();
        sender.send(7_u32);
        assert_eq!(pollster::block_on(receiver), Some(7));
    }

    #[test]
    fn dropped_sender_gives_none() {
        let (sender, receiver) = channel::<u32>();
        drop(sender);
        assert_eq!(pollster::block_on(receiver), None);
    }

    #[test]
    fn send_from_other_thread_wakes_receiver() {
        let (sender, receiver) = channel();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            sender.send("done");
        });
        assert_eq!(pollster::block_on(receiver), Some("done"));
        handle.join().expect("sender thread panicked");
    }
}
