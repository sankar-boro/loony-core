use std::cell::{UnsafeCell};
use std::fmt;
use std::future::Future;
use std::sync::{
    atomic::{AtomicUsize},
    Arc,
};

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::{TaskQueue, WAITING};

pub struct AtomicFuture {
    pub(crate) queue: Arc<TaskQueue>,
    pub(crate) status: AtomicUsize,
    pub(crate) future: UnsafeCell<BoxFuture<'static, ()>>,
}

impl fmt::Debug for AtomicFuture {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "AtomicFuture".fmt(f)
    }
}

unsafe impl Send for AtomicFuture {}
unsafe impl Sync for AtomicFuture {}

impl AtomicFuture {
    pub(crate) fn from_basic<F: Future<Output = ()> + Send + 'static>(fut: F, queue: Arc<TaskQueue>) -> Arc<AtomicFuture> {
        Arc::new(AtomicFuture {
            queue,
            status: AtomicUsize::new(WAITING),
            future: UnsafeCell::new(fut.boxed()),
        })
    }

    pub(crate) fn from_boxed(future: BoxFuture<'static, ()>, queue: Arc<TaskQueue>) -> Arc<AtomicFuture> {
        Arc::new(AtomicFuture {
            queue,
            status: AtomicUsize::new(WAITING),
            future: UnsafeCell::new(future),
        })
    }
}
