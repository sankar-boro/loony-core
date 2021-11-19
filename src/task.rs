use std::mem::{forget, ManuallyDrop};
use std::sync::{
    atomic::{Ordering::SeqCst},
    Arc,
};
use std::future::Future;
use std::task::{Context, Poll};

use crossbeam::channel;
use futures::FutureExt;
use futures::future::BoxFuture;

use crate::{WAITING, POLLING, REPOLL, COMPLETE};
use crate::atomics::AtomicFuture;
use crate::raw::waker;

#[derive(Clone, Debug)]
#[repr(transparent)]
pub struct Task(pub Arc<AtomicFuture>);


#[inline]
pub unsafe fn task(future: *const AtomicFuture) -> Task {
    Task(Arc::from_raw(future))
}

#[inline]
pub unsafe fn clone_task(future: *const AtomicFuture) -> Task {
    let task = task(future);
    forget(task.clone());
    task
}

pub struct TaskQueue {
    pub(crate) tx: channel::Sender<Task>,
    pub(crate) rx: channel::Receiver<Task>,
}

impl Default for TaskQueue {
    fn default() -> TaskQueue {
        let (tx, rx) = channel::unbounded();
        TaskQueue { tx, rx }
    }
}

impl Task {
    #[inline]
    pub(crate) fn new<F: Future<Output = ()> + Send + 'static>(future: F, queue: Arc<TaskQueue>) -> Task {
        let future = AtomicFuture::from_basic(future, queue);
        let future: *const AtomicFuture = Arc::into_raw(future) as *const AtomicFuture;
        unsafe { task(future) }
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn new_boxed(future: BoxFuture<'static, ()>, queue: Arc<TaskQueue>) -> Task {
        let future: Arc<AtomicFuture> = AtomicFuture::from_boxed(future, queue);
        let future: *const AtomicFuture = Arc::into_raw(future) as *const AtomicFuture;
        unsafe { task(future) }
    }

    pub(crate) fn send(&self, data: Self) {
        self.0.queue.tx.send(data).expect("Ooops! Failed to send data");
    }

    pub(crate) fn polling_repoll(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(POLLING, REPOLL, SeqCst, SeqCst)
    }

    pub(crate) fn waiting_repoll(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(WAITING, REPOLL, SeqCst, SeqCst)
    }

    pub(crate) fn waiting_polling(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(WAITING, POLLING, SeqCst, SeqCst)
    }

    pub(crate) fn polling_waiting(&self) -> Result<usize, usize> {
        self
        .0
        .status
        .compare_exchange(POLLING,WAITING, SeqCst, SeqCst)
    }

    pub(crate) fn polling(&self) {
        self.0.status.store(POLLING, SeqCst);
    }

    pub(crate) fn complete(&self) {
        self.0.status.store(COMPLETE, SeqCst);
    }

    pub(crate) fn poll_task(&self, cx: &mut Context) -> Poll<()> {
        unsafe {
            (&mut *self.0.future.get()).poll_unpin(cx)
        }
    }

    #[inline]
    pub(crate) unsafe fn poll(self) {
        self.polling();
        let waker = ManuallyDrop::new(waker(&*self.0));
        let mut cx = Context::from_waker(&waker);
        loop {
            // This is the part where our function gets executed
            if let Poll::Ready(_) =  self.poll_task(&mut cx){
                break self.complete();
            }
            match self.polling_waiting()
            {
                Ok(_) => break,
                Err(_) => self.polling(),
            }
        }
    }
}
